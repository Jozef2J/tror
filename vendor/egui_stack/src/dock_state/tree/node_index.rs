use crate::SurfaceIndex;

/// Identifies a node inside a [`Tree`](crate::Tree): its slot in the tree's collection of nodes.
///
/// A node keeps its index while the tree changes around it, with two exceptions: the root is
/// always at [`NodeIndex::root`], so a node that becomes the root, or stops being it, moves.
/// [`Tree::parent`](crate::Tree::parent) and [`Node::children`](crate::Node::children) walk the
/// tree.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Deserialize, serde::Serialize),
    serde(default)
)]
pub struct NodeIndex(pub usize);

impl From<usize> for NodeIndex {
    #[inline(always)]
    fn from(index: usize) -> Self {
        NodeIndex(index)
    }
}

impl NodeIndex {
    /// Returns the index of the root node.
    ///
    /// In the context of a [`Tree`](crate::Tree), this will be the node that contains all other nodes.
    /// The root is always at index 0.
    ///
    /// # Examples
    ///
    /// Splitting the current tree in two.
    /// ```rust
    /// # use egui_stack::{DockState, NodeIndex};
    /// let mut dock_state = DockState::new(vec!["tab 1", "tab 2"]);
    /// let _ = dock_state.main_surface_mut().split_left(NodeIndex::root(), 0.5, vec!["tab 3", "tab 4"]);
    /// ```
    #[inline(always)]
    pub const fn root() -> Self {
        Self(0)
    }
}

/// A full path to locate a node in an entire dock state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Deserialize, serde::Serialize),
    serde(default)
)]
pub struct NodePath {
    /// Index of the surface owning the node.
    pub surface: SurfaceIndex,
    /// Index of the node in the surface tree.
    pub node: NodeIndex,
}

impl NodePath {
    /// Points to the root node of the main surface.
    pub const MAIN_ROOT: Self = Self {
        surface: SurfaceIndex::main(),
        node: NodeIndex::root(),
    };

    /// Creates a fully qualified new path to a node.
    pub const fn new(surface: SurfaceIndex, node: NodeIndex) -> Self {
        Self { surface, node }
    }
}
