//! A window's whole space as a tree of zones: which zone is where, how
//! big, within which bounds. The layout knows nothing about tabs: a zone
//! holds a dock by name ([`Content::Dock`]), the application's own
//! drawing ([`Content::App`]) or nothing ([`Content::Empty`]).
//!
//! This module is the model only: the tree ([`Layout`], [`Zone`]), the
//! constraints set from code ([`Constraints`]), what the user changed
//! ([`ZoneState`]), and the solver that turns them into rects
//! ([`Layout::solve`], [`Laid`]). [`LayoutArea`] draws it: each zone's
//! content through a [`ZoneViewer`], and sashes between zones that resize
//! them, painted by a [`LayoutPainter`].
//!
//! The solver works one split at a time along the split's axis. Folded,
//! lying-over and hidden zones take their fixed breadth first; the others
//! take their wanted size within their bounds, and the zone that grows
//! (the centre) takes the rest. When the rest is under the centre's least
//! size, the others give way, the lowest [`Priority`] first and, among
//! equals, the nearest to the centre first, each down to its least size
//! and never in proportion: the rule [`crate::sections::plan`] applies to
//! sections. If even the least sizes do not fit, zones that opted in
//! ([`Constraints::auto_fold`]) fold to their strip, lowest priority
//! first, and come back when the room does.
//!
//! ```
//! use egui_stack::egui::{Rect, pos2};
//! use egui_stack::layout::{Content, Dir, Layout, Size, Solve, Zone};
//!
//! let layout = Layout::new(Zone::split(
//!     "root",
//!     Dir::Horizontal,
//!     vec![
//!         Zone::leaf("left", Content::Dock("left".into()))
//!             .with(|c| c.default = Some(Size::Points(200.0))),
//!         Zone::leaf("centre", Content::Dock("centre".into())).with(|c| {
//!             c.grow = true;
//!             c.min = Some(Size::Points(300.0));
//!         }),
//!     ],
//! ));
//! let laid = layout.solve(Rect::from_min_size(pos2(0.0, 0.0), [400.0, 300.0].into()), &Solve::default());
//! // The centre keeps its least size: the side gives way.
//! assert_eq!(laid.rect("left").unwrap().width(), 100.0);
//! assert_eq!(laid.rect("centre").unwrap().width(), 300.0);
//! ```

mod area;
mod fold;
#[cfg(feature = "serde")]
mod saved;
mod solve;

pub use area::{
    DefaultLayoutPainter, LayoutArea, LayoutPainter, LayoutStyle, SashPaint, SnapGuidePaint,
    SnapKind, ZoneViewer,
};
pub use fold::{Dismiss, Flyout, FlyoutPaint, Inside, StripPaint};

#[cfg(feature = "serde")]
pub use saved::{LAYOUT_VERSION, VersionError};
pub use solve::{Laid, LaidZone, LayoutEvent, Shown, Solve};

use std::collections::BTreeMap;
use std::fmt;

/// A zone's name: stable, saved, chosen by the application ("left",
/// "centre", "status"). Names are unique in a layout.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Deserialize, serde::Serialize),
    serde(transparent)
)]
pub struct ZoneName(pub String);

impl fmt::Display for ZoneName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&str> for ZoneName {
    fn from(name: &str) -> Self {
        Self(name.to_owned())
    }
}

impl From<String> for ZoneName {
    fn from(name: String) -> Self {
        Self(name)
    }
}

impl PartialEq<str> for ZoneName {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}

/// The whole layout: one value, serialisable with the `serde` feature.
///
/// Saved, it holds the tree and each zone's [`ZoneState`]; constraints
/// stay the code's: after loading, [`Layout::reconcile`] takes them from
/// the tree the code builds, and adds or drops the zones the code added or
/// removed since.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Deserialize, serde::Serialize),
    serde(try_from = "saved::SavedLayout", into = "saved::SavedLayout")
)]
pub struct Layout {
    /// The zone holding all others; it takes the whole rect it is solved in.
    pub root: Zone,
    /// Each flyout's breadth, by its key ([`Flyout::key`]), once the user
    /// resized it.
    pub flyouts: BTreeMap<String, f32>,
}

/// One zone of the tree: a split of zones, or a leaf with content.
#[derive(Clone, Debug, PartialEq)]
pub struct Zone {
    /// Its name, unique in the layout.
    pub name: ZoneName,
    /// A split or a leaf.
    pub kind: ZoneKind,
    /// Its bounds and behaviour, set from code (never saved).
    pub constraints: Constraints,
    /// What the user changed (saved).
    pub state: ZoneState,
}

/// What a zone is.
#[derive(Clone, Debug, PartialEq)]
pub enum ZoneKind {
    /// Children side by side ([`Dir::Horizontal`]) or one above the other
    /// ([`Dir::Vertical`]).
    Split {
        /// The axis the children follow each other on.
        dir: Dir,
        /// The children, left to right or top to bottom.
        children: Vec<Zone>,
    },
    /// A zone with content and no children.
    Leaf {
        /// What it shows.
        content: Content,
    },
}

/// The axis a split's children follow each other on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Deserialize, serde::Serialize),
    serde(rename_all = "kebab-case")
)]
pub enum Dir {
    /// Side by side, left to right.
    #[default]
    Horizontal,
    /// One above the other, top to bottom.
    Vertical,
}

/// What a leaf zone shows.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Deserialize, serde::Serialize),
    serde(rename_all = "kebab-case")
)]
#[non_exhaustive]
pub enum Content {
    /// The `DockSpace` dock of that name.
    Dock(String),
    /// Anything the application draws: a toolbar, a status bar, a viewer.
    App,
    /// Room left empty.
    #[default]
    Empty,
}

/// A size along the parent split's axis.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Size {
    /// Points.
    Points(f32),
    /// A share of the parent split's length (0.25: a quarter); of the
    /// solved rect at the root.
    Share(f32),
}

impl Size {
    /// The size in points, in a parent of length `length`.
    pub fn points(self, length: f32) -> f32 {
        match self {
            Self::Points(points) => points,
            Self::Share(share) => share * length,
        }
    }
}

/// Who gives way first when room is short: `Low` before `Normal` before
/// `High`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Priority {
    /// Gives way first.
    Low,
    /// Gives way after `Low`.
    #[default]
    Normal,
    /// Gives way last.
    High,
}

/// Whether a zone may fold to a strip.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Fold {
    /// It never folds.
    #[default]
    Never,
    /// It folds to a strip, which keeps its place in the flow.
    ToStrip,
}

/// Whether a dragged item may be dropped in a zone. The layout only
/// carries it: the dock layer reads it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Drops {
    /// Drops are accepted (the dock's own rules still apply).
    #[default]
    Accept,
    /// Nothing is dropped here.
    Refuse,
}

/// A zone's bounds and behaviour, set from code; never saved.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct Constraints {
    /// The least size; 0 when `None`.
    pub min: Option<Size>,
    /// The largest size; none when `None`.
    pub max: Option<Size>,
    /// The size a zone starts with, before the user sizes it.
    pub default: Option<Size>,
    /// The zone takes what its siblings leave (the centre). One per split
    /// at most: the first one counts. It never folds.
    pub grow: bool,
    /// Who gives way first when room is short.
    pub priority: Priority,
    /// The least size includes what the content asked for last frame
    /// ([`Solve::content`]).
    pub fit_content: bool,
    /// Whether the zone may fold to a strip.
    pub fold: Fold,
    /// With [`Fold::ToStrip`], the zone folds by itself when even the least
    /// sizes do not fit, and comes back when the room does. Off by default.
    pub auto_fold: bool,
    /// Whether a dragged item may be dropped here.
    pub drops: Drops,
    /// While the zone is folded, a tab dropped on its strip joins the dock
    /// it holds ([`Content::Dock`]), if [`drops`](Self::drops) and the dock
    /// accept it ([`DockSpace::bind`](crate::DockSpace::bind)). Off by
    /// default: the strip refuses drops.
    pub strip_drops: bool,
}

/// What the user changed; saved with the layout.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Deserialize, serde::Serialize),
    serde(default)
)]
#[non_exhaustive]
pub struct ZoneState {
    /// The size the user gave, in points, kept even while the zone is
    /// shown smaller.
    pub size: Option<f32>,
    /// Folded to its strip.
    pub folded: bool,
    /// Open over its neighbour, next to its strip, without taking room.
    pub over: bool,
    /// Not shown, and taking no room.
    pub hidden: bool,
}

impl Zone {
    /// A leaf zone with default constraints and state.
    pub fn leaf(name: impl Into<ZoneName>, content: Content) -> Self {
        Self {
            name: name.into(),
            kind: ZoneKind::Leaf { content },
            constraints: Constraints::default(),
            state: ZoneState::default(),
        }
    }

    /// A split zone with default constraints and state.
    pub fn split(name: impl Into<ZoneName>, dir: Dir, children: Vec<Self>) -> Self {
        Self {
            name: name.into(),
            kind: ZoneKind::Split { dir, children },
            constraints: Constraints::default(),
            state: ZoneState::default(),
        }
    }

    /// The zone with its constraints changed by `constrain`.
    pub fn with(mut self, constrain: impl FnOnce(&mut Constraints)) -> Self {
        constrain(&mut self.constraints);
        self
    }

    /// The zone with its state changed by `change`.
    pub fn with_state(mut self, change: impl FnOnce(&mut ZoneState)) -> Self {
        change(&mut self.state);
        self
    }

    /// Its children: none for a leaf.
    pub fn children(&self) -> &[Self] {
        match &self.kind {
            ZoneKind::Split { children, .. } => children,
            ZoneKind::Leaf { .. } => &[],
        }
    }

    /// The zone named `name`: this one or one under it.
    pub fn find(&self, name: &str) -> Option<&Self> {
        if self.name == *name {
            return Some(self);
        }
        self.children().iter().find_map(|child| child.find(name))
    }

    /// Calls `visit` on this zone and every zone under it, parents first,
    /// mutably.
    pub fn visit_mut(&mut self, visit: &mut impl FnMut(&mut Self)) {
        visit(self);
        if let ZoneKind::Split { children, .. } = &mut self.kind {
            for child in children {
                child.visit_mut(visit);
            }
        }
    }

    /// The zone named `name`, mutably: this one or one under it.
    pub fn find_mut(&mut self, name: &str) -> Option<&mut Self> {
        if self.name == *name {
            return Some(self);
        }
        match &mut self.kind {
            ZoneKind::Split { children, .. } => {
                children.iter_mut().find_map(|child| child.find_mut(name))
            }
            ZoneKind::Leaf { .. } => None,
        }
    }

    /// Calls `visit` on this zone and every zone under it, parents first.
    pub fn visit(&self, visit: &mut impl FnMut(&Self)) {
        visit(self);
        for child in self.children() {
            child.visit(visit);
        }
    }
}

impl Layout {
    /// A layout of `root`.
    pub fn new(root: Zone) -> Self {
        Self {
            root,
            flyouts: BTreeMap::new(),
        }
    }

    /// Folds the zone named `name` to its strip, or unfolds it; either way
    /// it no longer lies over its neighbour. Says whether the zone exists.
    /// The zone that grows never folds: the solver ignores its state.
    pub fn set_folded(&mut self, name: &str, folded: bool) -> bool {
        self.zone_mut(name)
            .map(|zone| {
                zone.state.folded = folded;
                zone.state.over = false;
            })
            .is_some()
    }

    /// Opens the zone named `name` over its neighbour, next to its strip,
    /// without taking room (`over`), or folds it back to its strip. Says
    /// whether the zone exists.
    pub fn set_over(&mut self, name: &str, over: bool) -> bool {
        self.zone_mut(name)
            .map(|zone| {
                zone.state.over = over;
                zone.state.folded = !over;
            })
            .is_some()
    }

    /// A layout of one leaf, `name`, holding the dock of the same name.
    pub fn single(name: &str) -> Self {
        Self::new(Zone::leaf(name, Content::Dock(name.to_owned())))
    }

    /// The zone named `name`.
    pub fn zone(&self, name: &str) -> Option<&Zone> {
        self.root.find(name)
    }

    /// The zone named `name`, mutably.
    pub fn zone_mut(&mut self, name: &str) -> Option<&mut Zone> {
        self.root.find_mut(name)
    }

    /// Changes the constraints of the zone named `name`, as code may every
    /// frame; says whether the zone exists.
    pub fn constrain(&mut self, name: &str, constrain: impl FnOnce(&mut Constraints)) -> bool {
        self.zone_mut(name)
            .map(|zone| constrain(&mut zone.constraints))
            .is_some()
    }

    /// Every zone's name, parents first.
    pub fn names(&self) -> Vec<ZoneName> {
        names_under(&self.root)
    }

    /// Makes a layout loaded from storage agree with `code`, the layout the
    /// application builds: every zone takes its constraints from `code`,
    /// zones `code` no longer has are dropped, and zones only `code` has are
    /// added under the same parent, after the same sibling as in `code`,
    /// with their state from `code`; the zones under a dropped split take
    /// its place. The loaded tree's shape, directions, contents and states
    /// are kept otherwise: a zone the code moved elsewhere stays where it
    /// was saved. A zone that is a leaf in one
    /// and a split in the other takes `code`'s zone, keeping its loaded
    /// state. When the roots' names differ, the loaded layout is replaced
    /// by `code`.
    pub fn reconcile(&mut self, code: &Self) -> Repair {
        let before = self.names();
        if self.root.name != code.root.name {
            *self = code.clone();
        } else {
            let in_code = code.names();
            drop_missing(&mut self.root, &in_code);
            follow_kinds(&mut self.root, &code.root);
            add_missing(&mut self.root, &code.root);
            let root = &code.root;
            self.root.visit_mut(&mut |zone| {
                if let Some(code) = root.find(&zone.name.0) {
                    zone.constraints = code.constraints;
                }
            });
        }
        let after = self.names();
        Repair {
            added: after
                .iter()
                .filter(|n| !before.contains(n))
                .cloned()
                .collect(),
            removed: before.into_iter().filter(|n| !after.contains(n)).collect(),
        }
    }
}

/// What [`Layout::reconcile`] changed in the loaded tree.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Repair {
    /// Zones the code has that the loaded layout had not, parents first.
    pub added: Vec<ZoneName>,
    /// Zones the loaded layout had that the code has not.
    pub removed: Vec<ZoneName>,
}

impl Repair {
    /// Whether the loaded tree already matched the code's.
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty()
    }
}

/// Drops from `zone`'s children, at any depth, the zones not in `keep`; the
/// kept zones under a dropped split take its place.
fn drop_missing(zone: &mut Zone, keep: &[ZoneName]) {
    if let ZoneKind::Split { children, .. } = &mut zone.kind {
        for mut child in std::mem::take(children) {
            drop_missing(&mut child, keep);
            if keep.contains(&child.name) {
                children.push(child);
            } else if let ZoneKind::Split {
                children: lifted, ..
            } = child.kind
            {
                children.extend(lifted);
            }
        }
    }
}

/// A copy of `code` without the zones under it that `loaded` has.
fn without_loaded(code: &Zone, loaded: &[ZoneName]) -> Zone {
    let mut zone = code.clone();
    drop_loaded(&mut zone, loaded);
    zone
}

fn drop_loaded(zone: &mut Zone, loaded: &[ZoneName]) {
    if let ZoneKind::Split { children, .. } = &mut zone.kind {
        children.retain(|child| !loaded.contains(&child.name));
        for child in children {
            drop_loaded(child, loaded);
        }
    }
}

/// Gives the loaded zones that are a leaf in one tree and a split in the
/// other the code's kind (and children), keeping their state.
fn follow_kinds(root: &mut Zone, code: &Zone) {
    let mut mismatched = Vec::new();
    root.visit(&mut |zone| {
        if let Some(code) = code.find(&zone.name.0) {
            let same = matches!(
                (&zone.kind, &code.kind),
                (ZoneKind::Split { .. }, ZoneKind::Split { .. })
                    | (ZoneKind::Leaf { .. }, ZoneKind::Leaf { .. })
            );
            if !same {
                mismatched.push(zone.name.clone());
            }
        }
    });
    for name in mismatched {
        let Some(zone) = root.find_mut(&name.0) else {
            continue;
        };
        // Its loaded children go with its loaded kind.
        zone.kind = ZoneKind::Leaf {
            content: Content::Empty,
        };
        let Some(code) = code.find(&name.0) else {
            continue;
        };
        let loaded = names_under(root);
        let kind = without_loaded(code, &loaded).kind;
        if let Some(zone) = root.find_mut(&name.0) {
            zone.kind = kind;
        }
    }
}

/// Adds the zones only `code` has, under the same parent and after the
/// same sibling as in `code` (first when none is loaded).
fn add_missing(root: &mut Zone, code: &Zone) {
    for (at, child) in code.children().iter().enumerate() {
        let loaded = names_under(root);
        if !loaded.contains(&child.name) {
            let added = without_loaded(child, &loaded);
            if let Some(ZoneKind::Split { children, .. }) =
                root.find_mut(&code.name.0).map(|parent| &mut parent.kind)
            {
                let place = code.children()[..at]
                    .iter()
                    .rev()
                    .find_map(|before| children.iter().position(|c| c.name == before.name))
                    .map_or(0, |i| i + 1);
                children.insert(place, added);
            }
        }
        add_missing(root, child);
    }
}

fn names_under(zone: &Zone) -> Vec<ZoneName> {
    let mut names = Vec::new();
    zone.visit(&mut |under| names.push(under.name.clone()));
    names
}

#[cfg(test)]
mod tests;
