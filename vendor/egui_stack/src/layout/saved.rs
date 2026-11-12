//! A layout as saved: the tree and each zone's state, versioned; never the
//! constraints, which the code re-asserts ([`Layout::reconcile`]).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::{Constraints, Content, Dir, Layout, Zone, ZoneKind, ZoneName, ZoneState};

/// The version a layout is saved with. A saved layout of a later version
/// does not load ([`VersionError`]).
pub const LAYOUT_VERSION: u32 = 1;

/// A saved layout's version is newer than this egui_stack reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("layout saved with version {found}, newer than {LAYOUT_VERSION}")]
pub struct VersionError {
    /// The version found.
    pub found: u32,
}

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct SavedLayout {
    version: u32,
    root: SavedZone,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    flyouts: BTreeMap<String, f32>,
}

#[derive(Clone, Serialize, Deserialize)]
struct SavedZone {
    name: ZoneName,
    #[serde(flatten)]
    kind: SavedKind,
    #[serde(default)]
    state: ZoneState,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum SavedKind {
    Split { dir: Dir, children: Vec<SavedZone> },
    Leaf { content: Content },
}

impl From<Layout> for SavedLayout {
    fn from(layout: Layout) -> Self {
        Self {
            version: LAYOUT_VERSION,
            root: layout.root.into(),
            flyouts: layout.flyouts,
        }
    }
}

impl From<Zone> for SavedZone {
    fn from(zone: Zone) -> Self {
        Self {
            name: zone.name,
            kind: match zone.kind {
                ZoneKind::Split { dir, children } => SavedKind::Split {
                    dir,
                    children: children.into_iter().map(Into::into).collect(),
                },
                ZoneKind::Leaf { content } => SavedKind::Leaf { content },
            },
            state: zone.state,
        }
    }
}

impl TryFrom<SavedLayout> for Layout {
    type Error = VersionError;

    fn try_from(saved: SavedLayout) -> Result<Self, VersionError> {
        if saved.version > LAYOUT_VERSION {
            return Err(VersionError {
                found: saved.version,
            });
        }
        let mut layout = Self::new(saved.root.into());
        layout.flyouts = saved.flyouts;
        Ok(layout)
    }
}

impl From<SavedZone> for Zone {
    fn from(saved: SavedZone) -> Self {
        Self {
            name: saved.name,
            kind: match saved.kind {
                SavedKind::Split { dir, children } => ZoneKind::Split {
                    dir,
                    children: children.into_iter().map(Into::into).collect(),
                },
                SavedKind::Leaf { content } => ZoneKind::Leaf { content },
            },
            constraints: Constraints::default(),
            state: saved.state,
        }
    }
}
