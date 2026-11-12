use egui::Rect;

use crate::TabIndex;

mod grid;
mod leaf;
mod linear;
pub(crate) mod shares;
pub use grid::{GridLayout, GridNode};
pub use leaf::LeafNode;
pub use linear::{LinearDir, LinearNode};

/// Represents an abstract node of a [`Tree`](crate::Tree).
///
/// More kinds of containers may come: the enum is `#[non_exhaustive]`, so a `match` outside
/// this crate needs a wildcard arm, and [`Node::children`] walks any container.
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[non_exhaustive]
pub enum Node<Tab> {
    /// Empty node.
    Empty,

    /// Contains the actual tabs.
    Leaf(LeafNode<Tab>),

    /// Parent node: two or more children in a row or a column, each with a share of the space.
    Linear(LinearNode),

    /// Parent node: two or more children in rows and columns, each column and each row with a
    /// share of the space.
    Grid(GridNode),
}

impl<Tab> Node<Tab> {
    /// Constructs a leaf node with a given `tab`.
    #[inline(always)]
    pub fn leaf(tab: Tab) -> Self {
        Self::Leaf(LeafNode::new(vec![tab]))
    }

    /// Get immutable access to the leaf data of this node, if it contains any (i.e is a leaf)
    pub fn get_leaf(&self) -> Option<&LeafNode<Tab>> {
        match self {
            Node::Leaf(leaf_node) => Some(leaf_node),
            _ => None,
        }
    }

    /// Get mutable access to the leaf data of this node, if it contains any (i.e is a leaf)
    pub fn get_leaf_mut(&mut self) -> Option<&mut LeafNode<Tab>> {
        match self {
            Node::Leaf(leaf_node) => Some(leaf_node),
            _ => None,
        }
    }

    /// Constructs a leaf node with a given list of `tabs`.
    #[inline(always)]
    pub const fn leaf_with(tabs: Vec<Tab>) -> Self {
        Self::Leaf(LeafNode::new(tabs))
    }

    /// Sets the area occupied by the node.
    ///
    /// If the node is a ``Node::Empty``, this will do nothing.
    #[inline]
    pub fn set_rect(&mut self, new_rect: Rect) {
        match self {
            Self::Empty => (),
            Self::Leaf(leaf) => leaf.set_rect(new_rect),
            Self::Linear(linear) => linear.set_rect(new_rect),
            Self::Grid(grid) => grid.set_rect(new_rect),
        }
    }

    /// Get a [`Rect`] occupied by the node, could be used e.g. to draw a highlight rect around a node.
    ///
    /// Returns [`None`] if node is of the [`Empty`](Node::Empty) variant.
    #[inline]
    pub fn rect(&self) -> Option<Rect> {
        match self {
            Self::Empty => None,
            Self::Leaf(leaf) => Some(leaf.rect()),
            Self::Linear(linear) => Some(linear.rect()),
            Self::Grid(grid) => Some(grid.rect()),
        }
    }

    /// Returns `true` if the node is a [`Empty`](Node::Empty), otherwise `false`.
    #[inline(always)]
    pub const fn is_empty(&self) -> bool {
        matches!(self, Self::Empty)
    }

    /// Returns `true` if the node is a [`Leaf`](Node::Leaf), otherwise `false`.
    #[inline(always)]
    pub const fn is_leaf(&self) -> bool {
        matches!(self, Self::Leaf { .. })
    }

    /// Returns `true` if the node is a [`Linear`](Node::Linear) laying out its children
    /// side by side, otherwise `false`.
    #[inline(always)]
    pub const fn is_horizontal(&self) -> bool {
        matches!(
            self,
            Self::Linear(LinearNode {
                dir: LinearDir::Horizontal,
                ..
            })
        )
    }

    /// Returns `true` if the node is a [`Linear`](Node::Linear) laying out its children one
    /// above the other, otherwise `false`.
    #[inline(always)]
    pub const fn is_vertical(&self) -> bool {
        matches!(
            self,
            Self::Linear(LinearNode {
                dir: LinearDir::Vertical,
                ..
            })
        )
    }

    /// Returns `true` if the node has children, otherwise `false`.
    #[inline(always)]
    pub const fn is_parent(&self) -> bool {
        matches!(self, Self::Linear(_) | Self::Grid(_))
    }

    /// Returns `true` if the node is a [`Grid`](Node::Grid).
    #[inline(always)]
    pub const fn is_grid(&self) -> bool {
        matches!(self, Self::Grid(_))
    }

    /// Get immutable access to the grid data of this node, if it is a [`Grid`](Node::Grid).
    pub fn get_grid(&self) -> Option<&GridNode> {
        match self {
            Node::Grid(grid) => Some(grid),
            _ => None,
        }
    }

    /// Get mutable access to the grid data of this node, if it is a [`Grid`](Node::Grid).
    pub fn get_grid_mut(&mut self) -> Option<&mut GridNode> {
        match self {
            Node::Grid(grid) => Some(grid),
            _ => None,
        }
    }

    /// Get immutable access to the linear data of this node, if it is a [`Linear`](Node::Linear).
    pub fn get_linear(&self) -> Option<&LinearNode> {
        match self {
            Node::Linear(linear) => Some(linear),
            _ => None,
        }
    }

    /// Get mutable access to the linear data of this node, if it is a [`Linear`](Node::Linear).
    pub fn get_linear_mut(&mut self) -> Option<&mut LinearNode> {
        match self {
            Node::Linear(linear) => Some(linear),
            _ => None,
        }
    }

    /// The children of this node: none unless it is a [`Linear`](Node::Linear) or a
    /// [`Grid`](Node::Grid).
    pub fn children(&self) -> &[crate::NodeIndex] {
        match self {
            Node::Linear(linear) => linear.children(),
            Node::Grid(grid) => grid.children(),
            _ => &[],
        }
    }

    /// The children of a container, to replace one in place.
    pub(crate) fn children_mut(&mut self) -> Option<&mut Vec<crate::NodeIndex>> {
        match self {
            Node::Linear(linear) => Some(&mut linear.children),
            Node::Grid(grid) => Some(&mut grid.children),
            _ => None,
        }
    }

    /// Returns `true` if the node is collapsed, otherwise `false`.
    #[inline(always)]
    pub fn is_collapsed(&self) -> bool {
        match self {
            Node::Leaf(leaf) => leaf.collapsed,
            Node::Linear(linear) => linear.fully_collapsed,
            Node::Grid(grid) => grid.fully_collapsed,
            Node::Empty => false,
        }
    }

    /// Returns the number of layers of collapsed leaf subnodes.
    pub fn collapsed_leaf_count(&self) -> i32 {
        match self {
            Node::Linear(linear) => linear.collapsed_leaf_count,
            Node::Grid(grid) => grid.collapsed_leaf_count,
            Node::Leaf(leaf) => {
                if leaf.collapsed {
                    1
                } else {
                    0
                }
            }
            Node::Empty => 0,
        }
    }

    /// Provides an immutable slice of the tabs inside this node.
    ///
    /// Returns [`None`] if the node is not a [`Leaf`](Node::Leaf).
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use egui_stack::{DockState, NodeIndex};
    /// let mut dock_state = DockState::new(vec![1, 2, 3, 4, 5, 6]);
    /// assert!(dock_state.main_surface().root_node().unwrap().tabs().unwrap().contains(&4));
    /// ```
    #[inline]
    pub fn tabs(&self) -> Option<&[Tab]> {
        match self {
            Node::Leaf(leaf) => Some(leaf.tabs()),
            _ => None,
        }
    }

    /// Provides an mutable slice of the tabs inside this node.
    ///
    /// Returns [`None`] if the node is not a [`Leaf`](Node::Leaf).
    ///
    /// # Examples
    ///
    /// Modifying tabs inside a node:
    /// ```rust
    /// # use egui_stack::{DockState, NodeIndex};
    /// let mut dock_state = DockState::new(vec![1, 2, 3, 4, 5, 6]);
    /// let mut tabs = dock_state
    ///     .main_surface_mut()
    ///     .root_node_mut()
    ///     .unwrap()
    ///     .tabs_mut()
    ///     .unwrap();
    ///
    /// tabs[0] = 7;
    /// tabs[5] = 8;
    ///
    /// assert_eq!(&tabs, &[7, 2, 3, 4, 5, 8]);
    /// ```
    #[inline]
    pub fn tabs_mut(&mut self) -> Option<&mut [Tab]> {
        match self {
            Node::Leaf(leaf) => Some(leaf.tabs_mut()),
            _ => None,
        }
    }

    /// Returns an [`Iterator`] of tabs in this node.
    ///
    /// If this node is not a [`Leaf`](Self::Leaf), then the returned [`Iterator`] will be empty.
    #[inline]
    pub fn iter_tabs(&self) -> impl Iterator<Item = &Tab> {
        match self.tabs() {
            Some(tabs) => tabs.iter(),
            None => core::slice::Iter::default(),
        }
    }

    /// Returns an [`Iterator`] of tabs in this node with their corresponding [`TabIndex`].
    pub fn iter_tabs_indexed(&self) -> impl Iterator<Item = (TabIndex, &Tab)> {
        self.iter_tabs()
            .enumerate()
            .map(|(index, tab)| (TabIndex(index), tab))
    }

    /// Returns a mutable [`Iterator`] of tabs in this node.
    ///
    /// If this node is not a [`Leaf`](Self::Leaf), then the returned [`Iterator`] will be empty.
    #[inline]
    pub fn iter_tabs_mut(&mut self) -> impl Iterator<Item = &mut Tab> {
        match self.tabs_mut() {
            Some(tabs) => tabs.iter_mut(),
            None => core::slice::IterMut::default(),
        }
    }

    /// Returns a mutable [`Iterator`] of tabs in this node with their corresponding [`TabIndex`].
    pub fn iter_tabs_mut_indexed(&mut self) -> impl Iterator<Item = (TabIndex, &mut Tab)> {
        self.iter_tabs_mut()
            .enumerate()
            .map(|(index, tab)| (TabIndex(index), tab))
    }

    /// Adds `tab` to the node and sets it as the active tab.
    ///
    /// # Panics
    ///
    /// If the new capacity of `tabs` exceeds `isize::MAX` bytes.
    ///
    /// If `self` is not a [`Leaf`](Node::Leaf) node.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use egui_stack::{DockState, NodeIndex};
    /// let mut dock_state = DockState::new(vec!["a tab"]);
    /// assert_eq!(dock_state.main_surface().root_node().unwrap().tabs_count(), 1);
    ///
    /// dock_state.main_surface_mut().root_node_mut().unwrap().append_tab("another tab");
    /// assert_eq!(dock_state.main_surface().root_node().unwrap().tabs_count(), 2);
    /// ```
    #[track_caller]
    #[inline]
    pub fn append_tab(&mut self, tab: Tab) {
        match self {
            Node::Leaf(leaf) => leaf.append_tab(tab),
            _ => panic!("node was not a leaf"),
        }
    }

    /// Sets the collapsing state of the node.
    ///
    /// # Panics
    ///
    /// Panics if `self` is an [`Empty`](Node::Empty) node.
    #[inline]
    pub fn set_collapsed(&mut self, collapsed: bool) {
        match self {
            Node::Leaf(leaf) => leaf.collapsed = collapsed,
            Node::Linear(linear) => linear.fully_collapsed = collapsed,
            Node::Grid(grid) => grid.fully_collapsed = collapsed,
            Node::Empty => panic!("node was empty"),
        }
    }

    /// Returns `true` if the node is a leaf with a hidden tab bar, otherwise `false`.
    #[inline(always)]
    pub fn is_tab_bar_hidden(&self) -> bool {
        match self {
            Node::Leaf(leaf) => leaf.tab_bar_hidden,
            _ => false,
        }
    }

    /// Sets the tab bar hidden state of the node.
    ///
    /// # Panics
    ///
    /// Panics if `self` is not a [`Leaf`](Node::Leaf) node.
    #[inline]
    pub fn set_tab_bar_hidden(&mut self, tab_bar_hidden: bool) {
        match self {
            Node::Leaf(leaf) => leaf.tab_bar_hidden = tab_bar_hidden,
            _ => panic!("node was not a leaf"),
        }
    }

    /// Sets the number of layers of collapsed leaf subnodes.
    ///
    /// # Panics
    ///
    /// Panics if `self` is not a [`Linear`](Node::Linear) or a [`Grid`](Node::Grid) node.
    #[inline]
    pub fn set_collapsed_leaf_count(&mut self, count: i32) {
        match self {
            Node::Linear(linear) => linear.collapsed_leaf_count = count,
            Node::Grid(grid) => grid.collapsed_leaf_count = count,
            _ => panic!("node was not a container"),
        }
    }

    /// Adds a `tab` to the node.
    ///
    /// # Panics
    ///
    /// Panics if the new capacity of `tabs` exceeds `isize::MAX` bytes, or `index > tabs_count()`.
    #[track_caller]
    #[inline]
    pub fn insert_tab(&mut self, index: TabIndex, tab: Tab) {
        match self {
            Node::Leaf(leaf) => leaf.insert_tab(index, tab),
            _ => panic!("node was not a leaf!"),
        }
    }

    /// Removes a tab at given `index` from the node.
    /// Returns the removed tab if the node is a `Leaf`, or `None` otherwise.
    ///
    /// # Panics
    ///
    /// Panics if `index` is out of bounds.
    #[inline]
    pub fn remove_tab(&mut self, tab_index: TabIndex) -> Option<Tab> {
        match self {
            Node::Leaf(leaf) => leaf.remove_tab(tab_index),
            _ => None,
        }
    }

    /// Gets the number of tabs in the node.
    #[inline]
    pub fn tabs_count(&self) -> usize {
        match self {
            Node::Leaf(leaf) => leaf.tabs.len(),
            _ => Default::default(),
        }
    }

    /// Returns a new [`Node`] while mapping and filtering the tab type.
    /// If this [`Node`] remains empty, it will change to [`Node::Empty`].
    pub fn filter_map_tabs<F, NewTab>(&self, function: F) -> Node<NewTab>
    where
        F: FnMut(&Tab) -> Option<NewTab>,
    {
        match self {
            Node::Leaf(leaf) => {
                let LeafNode {
                    rect,
                    viewport,
                    tabs,
                    active,
                    scroll,
                    collapsed,
                    tab_bar_hidden,
                } = leaf;
                let tabs: Vec<_> = tabs.iter().filter_map(function).collect();
                if tabs.is_empty() {
                    Node::Empty
                } else {
                    Node::Leaf(LeafNode {
                        rect: *rect,
                        viewport: *viewport,
                        tabs,
                        active: *active,
                        scroll: *scroll,
                        collapsed: *collapsed,
                        tab_bar_hidden: *tab_bar_hidden,
                    })
                }
            }
            Node::Empty => Node::Empty,
            Node::Linear(linear) => Node::Linear(linear.clone()),
            Node::Grid(grid) => Node::Grid(grid.clone()),
        }
    }

    /// Returns a new [`Node`] while mapping the tab type.
    pub fn map_tabs<F, NewTab>(&self, mut function: F) -> Node<NewTab>
    where
        F: FnMut(&Tab) -> NewTab,
    {
        self.filter_map_tabs(move |tab| Some(function(tab)))
    }

    /// Returns a new [`Node`] while filtering the tab type.
    /// If this [`Node`] remains empty, it will change to [`Node::Empty`].
    pub fn filter_tabs<F>(&self, mut predicate: F) -> Node<Tab>
    where
        F: FnMut(&Tab) -> bool,
        Tab: Clone,
    {
        self.filter_map_tabs(move |tab| predicate(tab).then(|| tab.clone()))
    }

    /// Removes all tabs for which `predicate` returns `false`.
    /// If this [`Node`] remains empty, it will change to [`Node::Empty`].
    pub fn retain_tabs<F>(&mut self, predicate: F)
    where
        F: FnMut(&mut Tab) -> bool,
    {
        if let Node::Leaf(leaf) = self {
            leaf.retain_tabs(predicate);
            if leaf.tabs.is_empty() {
                *self = Node::Empty;
            }
        }
    }
}
