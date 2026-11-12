//! Tree representing the relationships between [`Node`]s.
//!
//! # Implementation details
//!
//! The nodes are stored in a [`Vec`] indexed by [`NodeIndex`]. The root is always at index
//! *0*; a [`Linear`](Node::Linear) or [`Grid`](Node::Grid) node lists its children's indices. Free slots hold
//! [`Empty`](Node::Empty) nodes and are reused.

/// Iterates over all tabs in a [`Tree`].
pub mod tab_iter;

/// Identifies a tab within a [`Node`].
pub mod tab_index;

/// Represents an abstract node of a [`Tree`].
pub mod node;

/// Wrapper around indices to the collection of nodes inside a [`Tree`].
pub mod node_index;

#[cfg(feature = "serde")]
mod legacy;

use std::{
    collections::{HashSet, VecDeque},
    fmt,
    ops::{Index, IndexMut},
    slice::{Iter, IterMut},
};

use egui::Rect;
pub use node::LeafNode;
pub use node::Node;
pub use node::{GridLayout, GridNode, LinearDir, LinearNode};
pub use node_index::{NodeIndex, NodePath};
pub use tab_index::{TabIndex, TabPath};
pub use tab_iter::TabIter;

use crate::{Error, Result, SurfaceIndex};

// ----------------------------------------------------------------------------

/// Direction in which a new node is created relatively to the parent node at which the split occurs.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[allow(missing_docs)]
pub enum Split {
    Left,
    Right,
    Above,
    Below,
}

impl Split {
    /// The direction of a linear node holding the split node and the new one.
    pub const fn dir(self) -> LinearDir {
        match self {
            Split::Left | Split::Right => LinearDir::Horizontal,
            Split::Above | Split::Below => LinearDir::Vertical,
        }
    }

    /// Whether the new node comes before the split one: left of it or above it.
    pub const fn new_first(self) -> bool {
        matches!(self, Split::Left | Split::Above)
    }

    /// Returns whether the split is vertical.
    pub const fn is_top_bottom(self) -> bool {
        matches!(self, Split::Above | Split::Below)
    }

    /// Returns whether the split is horizontal.
    pub const fn is_left_right(self) -> bool {
        matches!(self, Split::Left | Split::Right)
    }
}

/// Specify how a tab should be added to a Node.
pub enum TabInsert {
    /// Split the node in the given direction.
    Split(Split),

    /// Insert the tab at the given index.
    Insert(TabIndex),

    /// Append the tab to the node.
    Append,
}

/// The destination for a tab which is being moved.
pub enum TabDestination {
    /// Move to a new window with this rect.
    Window(Rect),

    /// Move to an existing node with this insertion.
    Node(NodePath, TabInsert),

    /// Move to an empty surface.
    EmptySurface(SurfaceIndex),
}

impl From<(NodePath, TabInsert)> for TabDestination {
    fn from(value: (NodePath, TabInsert)) -> TabDestination {
        TabDestination::Node(value.0, value.1)
    }
}

impl From<SurfaceIndex> for TabDestination {
    fn from(value: SurfaceIndex) -> TabDestination {
        TabDestination::EmptySurface(value)
    }
}

impl TabDestination {
    /// Returns if this tab destination is a [`Window`](TabDestination::Window).
    pub fn is_window(&self) -> bool {
        matches!(self, Self::Window(_))
    }
}

/// Tree representing the relationships between [`Node`]s.
///
/// # Implementation details
///
/// The nodes are stored in a [`Vec`] indexed by [`NodeIndex`]. The root is always at index
/// *0*. A [`Linear`](Node::Linear) node holds two or more children, side by side or one above
/// the other, each with a share of the space; a [`Grid`](Node::Grid) node holds two or more
/// children in rows and columns; [`Leaf`](Node::Leaf) nodes hold the tabs. Free slots hold
/// [`Empty`](Node::Empty) nodes.
///
/// Splitting a node whose parent already lays out its children in the split's direction adds
/// the new node to that parent, next to the split one, instead of nesting a new split: a row
/// or a column grows by one, and the separator between two neighbours resizes those two.
///
/// With the `serde` feature, trees saved by earlier versions, binary trees of `Horizontal`
/// and `Vertical` splits, load as linear nodes with the same layout.
#[derive(Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(
    feature = "serde",
    serde(
        from = "legacy::TreeRepr<Tab>",
        bound(deserialize = "Tab: serde::Deserialize<'de>")
    )
)]
pub struct Tree<Tab> {
    // The nodes, the root first.
    pub(super) nodes: Vec<Node<Tab>>,
    focused_node: Option<NodeIndex>,
    // Whether all subnodes of the tree is collapsed
    collapsed: bool,
    collapsed_leaf_count: i32,
}

impl<Tab> fmt::Debug for Tree<Tab> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Tree").finish_non_exhaustive()
    }
}

impl<Tab> Default for Tree<Tab> {
    fn default() -> Self {
        Self {
            nodes: Vec::new(),
            focused_node: None,
            collapsed: false,
            collapsed_leaf_count: 0,
        }
    }
}

impl<Tab> Index<NodeIndex> for Tree<Tab> {
    type Output = Node<Tab>;

    #[inline(always)]
    fn index(&self, index: NodeIndex) -> &Self::Output {
        &self.nodes[index.0]
    }
}

impl<Tab> IndexMut<NodeIndex> for Tree<Tab> {
    #[inline(always)]
    fn index_mut(&mut self, index: NodeIndex) -> &mut Self::Output {
        &mut self.nodes[index.0]
    }
}

impl<Tab> Tree<Tab> {
    /// Creates a new [`Tree`] with given `Vec` of `Tab`s in its root node.
    #[inline(always)]
    pub fn new(tabs: Vec<Tab>) -> Self {
        let root = Node::leaf_with(tabs);
        Self {
            nodes: vec![root],
            focused_node: None,
            collapsed: false,
            collapsed_leaf_count: 0,
        }
    }

    /// Returns the viewport [`Rect`] and the `Tab` inside the first leaf node,
    /// or `None` if no leaf exists in the [`Tree`].
    #[inline]
    pub fn find_active(&mut self) -> Option<(Rect, &mut Tab)> {
        self.nodes.iter_mut().find_map(|node| match node {
            Node::Leaf(leaf) => leaf
                .tabs
                .get_mut(leaf.active.0)
                .map(|tab| (leaf.viewport.to_owned(), tab)),
            _ => None,
        })
    }

    /// Returns the number of nodes in the [`Tree`].
    ///
    /// This includes [`Empty`](Node::Empty) nodes.
    #[inline(always)]
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Returns `true` if the number of nodes in the tree is 0, otherwise `false`.
    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Returns an [`Iterator`] of the underlying collection of nodes.
    ///
    /// This includes [`Empty`](Node::Empty) nodes.
    #[inline(always)]
    pub fn iter(&self) -> Iter<'_, Node<Tab>> {
        self.nodes.iter()
    }

    /// Returns [`IterMut`] of the underlying collection of nodes.
    ///
    /// This includes [`Empty`](Node::Empty) nodes.
    #[inline(always)]
    pub fn iter_mut(&mut self) -> IterMut<'_, Node<Tab>> {
        self.nodes.iter_mut()
    }

    /// Returns an [`Iterator`] of [`NodeIndex`] ordered in a breadth first manner, from the
    /// root: a parent always comes before its children. Free slots are skipped.
    pub(crate) fn breadth_first_index_iter(&self) -> impl Iterator<Item = NodeIndex> + use<Tab> {
        let mut order = Vec::new();
        if !self.nodes.is_empty() {
            let mut queue = VecDeque::from([NodeIndex::root()]);
            let mut seen = HashSet::new();
            while let Some(index) = queue.pop_front() {
                if !seen.insert(index) {
                    continue;
                }
                let Some(node) = self.nodes.get(index.0) else {
                    continue;
                };
                if node.is_empty() {
                    continue;
                }
                order.push(index);
                queue.extend(node.children().iter().copied());
            }
        }
        order.into_iter()
    }

    /// Returns the parent of `node`, or `None` for the root or a node that is not in the tree.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use egui_stack::{DockState, NodeIndex};
    /// let mut dock_state = DockState::new(vec!["tab 1"]);
    /// let tree = dock_state.main_surface_mut();
    /// let [a, b] = tree.split_right(NodeIndex::root(), 0.5, vec!["tab 2"]);
    /// assert_eq!(tree.parent(a), Some(NodeIndex::root()));
    /// assert_eq!(tree.parent(b), Some(NodeIndex::root()));
    /// assert_eq!(tree.parent(NodeIndex::root()), None);
    /// ```
    pub fn parent(&self, node: NodeIndex) -> Option<NodeIndex> {
        self.nodes
            .iter()
            .position(|n| n.children().contains(&node))
            .map(NodeIndex)
    }

    /// Puts `node` in a free slot and returns its index.
    fn alloc(&mut self, node: Node<Tab>) -> NodeIndex {
        match self
            .nodes
            .iter()
            .skip(1)
            .position(|n| n.is_empty())
            .map(|i| i + 1)
        {
            Some(index) => {
                self.nodes[index] = node;
                NodeIndex(index)
            }
            None => {
                self.nodes.push(node);
                NodeIndex(self.nodes.len() - 1)
            }
        }
    }

    /// Moves the node at `from` to the slot `to`, leaving `from` free; the focus follows it.
    /// The caller fixes the references to `from`.
    fn move_node(&mut self, from: NodeIndex, to: NodeIndex) {
        self.nodes[to.0] = std::mem::replace(&mut self.nodes[from.0], Node::Empty);
        if self.focused_node == Some(from) {
            self.focused_node = Some(to);
        }
    }

    /// Puts `new` in the place of `old` among the children of `parent`.
    fn replace_child(&mut self, parent: NodeIndex, old: NodeIndex, new: NodeIndex) {
        let children = self[parent]
            .children_mut()
            .expect("a parent is a container");
        let at = children
            .iter()
            .position(|&c| c == old)
            .expect("a child of its parent");
        children[at] = new;
    }

    /// Drops the free slots at the end of the collection.
    fn trim(&mut self) {
        while self.nodes.last().is_some_and(Node::is_empty) {
            self.nodes.pop();
        }
    }

    /// Returns an iterator over all tabs in arbitrary order.
    #[inline(always)]
    pub fn tabs(&self) -> TabIter<'_, Tab> {
        TabIter::new(self)
    }

    /// Counts and returns the number of tabs in the whole tree.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use egui_stack::{DockState, NodeIndex, TabIndex};
    /// let mut dock_state = DockState::new(vec!["node 1", "node 2", "node 3"]);
    /// assert_eq!(dock_state.main_surface().num_tabs(), 3);
    ///
    /// let [a, b] = dock_state.main_surface_mut().split_left(NodeIndex::root(), 0.5, vec!["tab 4", "tab 5"]);
    /// assert_eq!(dock_state.main_surface().num_tabs(), 5);
    ///
    /// dock_state.main_surface_mut().remove_leaf(a);
    /// assert_eq!(dock_state.main_surface().num_tabs(), 2);
    /// ```
    #[inline]
    pub fn num_tabs(&self) -> usize {
        let mut count = 0;
        for node in self.nodes.iter() {
            if let Node::Leaf(leaf) = node {
                count += leaf.tabs.len();
            }
        }
        count
    }

    /// Acquire a immutable borrow to the [`Node`] at the root of the tree.
    /// Returns [`None`] if the tree is empty.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use egui_stack::DockState;
    /// let mut dock_state = DockState::new(vec!["single tab"]);
    /// let root_node = dock_state.main_surface().root_node().unwrap();
    ///
    /// assert_eq!(root_node.tabs(), Some(["single tab"].as_slice()));
    /// ```
    pub fn root_node(&self) -> Option<&Node<Tab>> {
        self.nodes.first()
    }

    /// Acquire a mutable borrow to the [`Node`] at the root of the tree.
    /// Returns [`None`] if the tree is empty.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use egui_stack::{DockState, LeafNode};
    /// let mut dock_state = DockState::new(vec!["single tab"]);
    /// let root_node = dock_state.main_surface_mut().root_node_mut().unwrap();
    /// let root_as_leaf = root_node.get_leaf_mut().unwrap();
    /// root_as_leaf.tabs.push("partner tab");
    ///
    /// assert_eq!(root_node.tabs(), Some(["single tab", "partner tab"].as_slice()));
    /// ```
    pub fn root_node_mut(&mut self) -> Option<&mut Node<Tab>> {
        self.nodes.first_mut()
    }

    /// Creates two new nodes by splitting a given `parent` node and assigns them as its children. The first (old) node
    /// inherits content of the `parent` from before the split, and the second (new) gets the `tabs`.
    ///
    /// `fraction` (in range 0..=1) specifies how much of the `parent` node's area the old node will attempt to occupy
    /// after the split.
    ///
    /// The new node is placed relatively to the old node, in the direction specified by `split`.
    ///
    /// Returns the indices of the old node and the new node.
    ///
    /// # Panics
    ///
    /// If `fraction` isn't in range 0..=1.
    ///
    /// # Example
    ///
    /// ```rust
    /// # use egui_stack::{DockState, SurfaceIndex, NodeIndex, Split};
    /// let mut dock_state = DockState::new(vec!["tab 1", "tab 2"]);
    ///
    /// // At this point, the main surface only contains the leaf with tab 1 and 2.
    /// assert!(dock_state.main_surface().root_node().unwrap().is_leaf());
    ///
    /// // Split the node, giving 50% of the space to the new nodes and 50% to the old ones.
    /// let [old, new] = dock_state.main_surface_mut()
    ///     .split_tabs(NodeIndex::root(), Split::Below, 0.5, vec!["tab 3"]);
    ///
    /// assert!(dock_state.main_surface().root_node().unwrap().is_parent());
    /// assert!(dock_state[SurfaceIndex::main()][old].is_leaf());
    /// assert!(dock_state[SurfaceIndex::main()][new].is_leaf());
    /// ```
    #[inline(always)]
    pub fn split_tabs(
        &mut self,
        parent: NodeIndex,
        split: Split,
        fraction: f32,
        tabs: Vec<Tab>,
    ) -> [NodeIndex; 2] {
        self.split(parent, split, fraction, Node::leaf_with(tabs))
    }

    /// Creates two new nodes by splitting a given `parent` node and assigns them as its children. The first (old) node
    /// inherits content of the `parent` from before the split, and the second (new) gets the `tabs`.
    ///
    /// This is a shorthand for using `split_tabs` with [`Split::Above`].
    ///
    /// `fraction` (in range 0..=1) specifies how much of the `parent` node's area the old node will attempt to occupy
    /// after the split.
    ///
    /// The new node is placed *above* the old node.
    ///
    /// Returns the indices of the old node and the new node.
    ///
    /// # Panics
    ///
    /// If `fraction` isn't in range 0..=1.
    #[inline(always)]
    pub fn split_above(
        &mut self,
        parent: NodeIndex,
        fraction: f32,
        tabs: Vec<Tab>,
    ) -> [NodeIndex; 2] {
        self.split(parent, Split::Above, fraction, Node::leaf_with(tabs))
    }

    /// Creates two new nodes by splitting a given `parent` node and assigns them as its children. The first (old) node
    /// inherits content of the `parent` from before the split, and the second (new) gets the `tabs`.
    ///
    /// This is a shorthand for using `split_tabs` with [`Split::Below`].
    ///
    /// `fraction` (in range 0..=1) specifies how much of the `parent` node's area the old node will attempt to occupy
    /// after the split.
    ///
    /// The new node is placed *below* the old node.
    ///
    /// Returns the indices of the old node and the new node.
    ///
    /// # Panics
    ///
    /// If `fraction` isn't in range 0..=1.
    #[inline(always)]
    pub fn split_below(
        &mut self,
        parent: NodeIndex,
        fraction: f32,
        tabs: Vec<Tab>,
    ) -> [NodeIndex; 2] {
        self.split(parent, Split::Below, fraction, Node::leaf_with(tabs))
    }

    /// Creates two new nodes by splitting a given `parent` node and assigns them as its children. The first (old) node
    /// inherits content of the `parent` from before the split, and the second (new) gets the `tabs`.
    ///
    /// This is a shorthand for using `split_tabs` with [`Split::Left`].
    ///
    /// `fraction` (in range 0..=1) specifies how much of the `parent` node's area the old node will attempt to occupy
    /// after the split.
    ///
    /// The new node is placed to the *left* of the old node.
    ///
    /// Returns the indices of the old node and the new node.
    ///
    /// # Panics
    ///
    /// If `fraction` isn't in range 0..=1.
    #[inline(always)]
    pub fn split_left(
        &mut self,
        parent: NodeIndex,
        fraction: f32,
        tabs: Vec<Tab>,
    ) -> [NodeIndex; 2] {
        self.split(parent, Split::Left, fraction, Node::leaf_with(tabs))
    }

    /// Creates two new nodes by splitting a given `parent` node and assigns them as its children. The first (old) node
    /// inherits content of the `parent` from before the split, and the second (new) gets the `tabs`.
    ///
    /// This is a shorthand for using `split_tabs` with [`Split::Right`].
    ///
    /// `fraction` (in range 0..=1) specifies how much of the `parent` node's area the old node will attempt to occupy
    /// after the split.
    ///
    /// The new node is placed to the *right* of the old node.
    ///
    /// Returns the indices of the old node and the new node.
    ///
    /// # Panics
    ///
    /// If `fraction` isn't in range 0..=1.
    #[inline(always)]
    pub fn split_right(
        &mut self,
        parent: NodeIndex,
        fraction: f32,
        tabs: Vec<Tab>,
    ) -> [NodeIndex; 2] {
        self.split(parent, Split::Right, fraction, Node::leaf_with(tabs))
    }

    /// Splits a given `parent` node in two: the old node keeps the content of `parent`, and the
    /// new node is `new`.
    ///
    /// `fraction` (in range 0..=1) specifies how much of the `parent` node's area the first of the
    /// two nodes (the left or the top one) will attempt to occupy after the split.
    ///
    /// The new node is placed relatively to the old node, in the direction specified by `split`.
    ///
    /// When the parent of `parent` already lays out its children in that direction, the new node
    /// joins it, next to the old one, and the two share the old node's space: no split is nested.
    /// When the parent is a [`Grid`](Node::Grid), the new node joins the grid, right after the
    /// old one (or right before it for [`Split::Left`] and [`Split::Above`]), and `fraction` is
    /// not used: the grid lays out its children again. Otherwise a new [`Linear`](Node::Linear) node takes the place of `parent` and holds the
    /// two.
    ///
    /// Returns the indices of the old node and the new node. The old node keeps its index unless
    /// it was the root.
    ///
    /// # Panics
    ///
    /// If `fraction` isn't in range 0..=1.
    ///
    /// If `new` is not a [`Leaf`](Node::Leaf) node.
    ///
    /// If `new` is a [`Leaf`](Node::Leaf) node without any tabs.
    ///
    /// If `parent` points to an [`Empty`](Node::Empty) node.
    ///
    /// # Example
    ///
    /// ```rust
    /// # use egui_stack::{DockState, SurfaceIndex, NodeIndex, Split, Node};
    /// let mut dock_state = DockState::new(vec!["tab 1", "tab 2"]);
    ///
    /// // At this point, the main surface only contains the leaf with tab 1 and 2.
    /// assert!(dock_state.main_surface().root_node().unwrap().is_leaf());
    ///
    /// // Splits the node, giving 50% of the space to the new nodes and 50% to the old ones.
    /// let [old, new] = dock_state.main_surface_mut()
    ///     .split(NodeIndex::root(), Split::Below, 0.5, Node::leaf_with(vec!["tab 3"]));
    ///
    /// assert!(dock_state.main_surface().root_node().unwrap().is_parent());
    /// assert!(dock_state[SurfaceIndex::main()][old].is_leaf());
    /// assert!(dock_state[SurfaceIndex::main()][new].is_leaf());
    ///
    /// // Splitting "tab 3" below again makes a column of three, not a nested split.
    /// let [_, newer] = dock_state.main_surface_mut()
    ///     .split(new, Split::Below, 0.5, Node::leaf_with(vec!["tab 4"]));
    /// let root = dock_state.main_surface().root_node().unwrap();
    /// assert_eq!(root.children(), &[old, new, newer]);
    /// ```
    pub fn split(
        &mut self,
        parent: NodeIndex,
        split: Split,
        fraction: f32,
        new: Node<Tab>,
    ) -> [NodeIndex; 2] {
        assert!((0.0..=1.0).contains(&fraction));
        assert!(!self[parent].is_empty());
        assert_ne!(new.tabs_count(), 0);

        let dir = split.dir();
        let shares = |total: f32| [total * fraction, total * (1.0 - fraction)];

        let index = if let Some(grand) = self.parent(parent)
            && self[grand].is_grid()
        {
            // Join the grid, next to the old node.
            let new_index = self.alloc(new);
            let Node::Grid(grid) = &mut self[grand] else {
                unreachable!()
            };
            let at = grid.position(parent).expect("a child of its parent");
            let at = if split.new_first() { at } else { at + 1 };
            grid.children.insert(at, new_index);
            grid.fit_shares();
            [parent, new_index]
        } else if let Some(grand) = self.parent(parent)
            && let Node::Linear(linear) = &self[grand]
            && linear.dir == dir
        {
            // Join the parent's row or column, sharing the old node's space.
            let new_index = self.alloc(new);
            let Node::Linear(linear) = &mut self[grand] else {
                unreachable!()
            };
            let at = linear.position(parent).expect("a child of its parent");
            let [a, b] = shares(linear.shares[at]);
            let children = if split.new_first() {
                [new_index, parent]
            } else {
                [parent, new_index]
            };
            linear.shares[at] = a;
            linear.shares.insert(at + 1, b);
            linear.children[at] = children[0];
            linear.children.insert(at + 1, children[1]);
            [parent, new_index]
        } else {
            // A new linear node takes the parent's place and holds the old and the new nodes.
            let collapsed = self[parent].is_collapsed();
            let collapsed_leaf_count = self[parent].collapsed_leaf_count();
            // The new nodes take free slots in their order on screen.
            let (old_index, new_index, holder) = if parent == NodeIndex::root() {
                let old = std::mem::replace(&mut self.nodes[0], Node::Empty);
                let (old_index, new_index) = if split.new_first() {
                    let new_index = self.alloc(new);
                    (self.alloc(old), new_index)
                } else {
                    let old_index = self.alloc(old);
                    (old_index, self.alloc(new))
                };
                if self.focused_node == Some(parent) {
                    self.focused_node = Some(old_index);
                }
                (old_index, new_index, NodeIndex::root())
            } else {
                // A placeholder keeps the slot taken until the holder is built.
                let holder = self.alloc(Node::Linear(LinearNode::default()));
                let grand = self
                    .parent(parent)
                    .expect("a node other than the root has one");
                self.replace_child(grand, parent, holder);
                (parent, self.alloc(new), holder)
            };
            let mut children = vec![old_index, new_index];
            if split.new_first() {
                children.reverse();
            }
            let mut linear = LinearNode::new(dir, children, shares(1.0).to_vec());
            linear.fully_collapsed = collapsed;
            linear.collapsed_leaf_count = collapsed_leaf_count;
            self.nodes[holder.0] = Node::Linear(linear);
            [old_index, new_index]
        };

        self.focused_node = Some(index[1]);
        self.node_update_collapsed(index[1]);

        index
    }

    /// Adds `new` to the [`Linear`](Node::Linear) or [`Grid`](Node::Grid) node `container`, as
    /// its child at `index`.
    ///
    /// In a linear node, the new child gets an even share: `1 / n` of the space, `n` counting
    /// it, and the other children keep their proportions in the rest. A grid lays out its
    /// children again; a new column or row gets the mean share of the others.
    ///
    /// Returns the index of the new node.
    ///
    /// # Panics
    ///
    /// If `container` is not a [`Linear`](Node::Linear) or [`Grid`](Node::Grid) node, or `index`
    /// is greater than its number of children.
    ///
    /// If `new` is not a [`Leaf`](Node::Leaf) node with tabs.
    ///
    /// # Example
    ///
    /// ```rust
    /// # use egui_stack::{DockState, NodeIndex, Node};
    /// let mut dock_state = DockState::new(vec!["a"]);
    /// let tree = dock_state.main_surface_mut();
    /// let [a, b] = tree.split_right(NodeIndex::root(), 0.5, vec!["b"]);
    /// let c = tree.insert_child(NodeIndex::root(), 2, Node::leaf("c"));
    ///
    /// let row = tree.root_node().unwrap().get_linear().unwrap();
    /// assert_eq!(row.children(), &[a, b, c]);
    /// assert!((row.fraction(2) - 1.0 / 3.0).abs() < 1e-6);
    /// ```
    pub fn insert_child(
        &mut self,
        container: NodeIndex,
        index: usize,
        new: Node<Tab>,
    ) -> NodeIndex {
        assert!(
            self[container].is_parent(),
            "the container is a linear or a grid node"
        );
        assert!(index <= self[container].children().len());
        assert_ne!(new.tabs_count(), 0);
        let new_index = self.alloc(new);
        match &mut self[container] {
            Node::Linear(linear) => {
                linear.normalize_shares();
                let n = linear.len() as f32;
                for share in &mut linear.shares {
                    *share *= n / (n + 1.0);
                }
                linear.children.insert(index, new_index);
                linear.shares.insert(index, 1.0 / (n + 1.0));
            }
            Node::Grid(grid) => {
                grid.children.insert(index, new_index);
                grid.fit_shares();
            }
            _ => unreachable!(),
        }
        self.focused_node = Some(new_index);
        self.node_update_collapsed(new_index);
        new_index
    }

    /// Immutably borrows a leaf node at the given path.
    ///
    /// Returns `Err` if the path is invalid or the node at the path is not a leaf.
    pub fn leaf(&self, node: NodeIndex) -> Result<&LeafNode<Tab>> {
        self.nodes
            .get(node.0)
            .ok_or(Error::InvalidNode)?
            .get_leaf()
            .ok_or(Error::NonLeafNode)
    }

    /// Mutably borrows a leaf node at the given index.
    ///
    /// Returns `Err` if the index is invalid or if the node is not a leaf.
    pub fn leaf_mut(&mut self, node: NodeIndex) -> Result<&mut LeafNode<Tab>> {
        self.nodes
            .get_mut(node.0)
            .ok_or(Error::InvalidNode)?
            .get_leaf_mut()
            .ok_or(Error::NonLeafNode)
    }

    /// The first leaf in the subtree of `top`, `top` included, depth first.
    fn first_leaf(&self, top: NodeIndex) -> Option<NodeIndex> {
        match self.nodes.get(top.0)? {
            Node::Leaf(_) => Some(top),
            node => node
                .children()
                .iter()
                .find_map(|&child| self.first_leaf(child)),
        }
    }

    /// Returns the viewport [`Rect`] and the `Tab` inside the focused leaf node or [`None`] if it does not exist.
    #[inline]
    pub fn find_active_focused(&mut self) -> Option<(Rect, &mut Tab)> {
        match self.focused_node.and_then(|idx| self.nodes.get_mut(idx.0)) {
            Some(Node::Leaf(leaf)) => leaf.active_focused(),
            _ => None,
        }
    }

    /// Gets the node index of currently focused leaf node; returns [`None`] when no leaf is focused.
    #[inline]
    pub fn focused_leaf(&self) -> Option<NodeIndex> {
        self.focused_node
    }

    /// Sets the currently focused leaf to `node_index` if the node at `node_index` is a leaf.
    ///
    /// This method will not never panic and instead removes focus from all nodes when given an invalid index.
    #[inline]
    pub fn set_focused_node(&mut self, node_index: NodeIndex) {
        self.focused_node = self
            .nodes
            .get(node_index.0)
            .filter(|node| node.is_leaf())
            .map(|_| node_index);
    }

    /// Removes the given node from the [`Tree`].
    ///
    /// Its siblings share its space, in proportion to their own. A parent left with one child
    /// gives way to that child.
    ///
    /// # Panics
    ///
    /// - If the tree is empty.
    /// - If the node at index `node` is not a [`Leaf`](Node::Leaf).
    pub fn remove_leaf(&mut self, node: NodeIndex) {
        assert!(!self.is_empty());
        assert!(self[node].is_leaf());

        let Some(parent) = self.parent(node) else {
            self.nodes.clear();
            self.focused_node = None;
            return;
        };

        if Some(node) == self.focused_node {
            // Focus the nearest leaf: a sibling's, then an uncle's, and so on up.
            self.focused_node = None;
            let mut child = node;
            let mut ancestor = Some(parent);
            while let Some(up) = ancestor {
                // The nearest siblings first.
                let siblings = self[up].children();
                let at = siblings.iter().position(|&c| c == child).unwrap_or(0);
                let mut order: Vec<usize> = (0..siblings.len()).filter(|&i| i != at).collect();
                order.sort_by_key(|&i| i.abs_diff(at));
                let found = order.into_iter().find_map(|i| self.first_leaf(siblings[i]));
                if found.is_some() {
                    self.focused_node = found;
                    break;
                }
                child = up;
                ancestor = self.parent(up);
            }
        }

        self[node] = Node::Empty;
        match &mut self[parent] {
            Node::Linear(linear) => {
                let at = linear.position(node).expect("a child of its parent");
                linear.children.remove(at);
                linear.shares.remove(at);
            }
            Node::Grid(grid) => {
                let at = grid.position(node).expect("a child of its parent");
                grid.children.remove(at);
                grid.fit_shares();
            }
            _ => unreachable!("a parent is a container"),
        }
        if self[parent].children().len() == 1 {
            self.give_way(parent);
        }
        self.trim();
    }

    /// Replaces the container `holder`, left with one child, by that child.
    fn give_way(&mut self, holder: NodeIndex) {
        let child = self[holder].children()[0];
        let grand = self.parent(holder);
        match grand {
            None => {
                // The root: the child moves into its slot, and its children stay where they are.
                self.move_node(child, holder);
            }
            Some(grand) => {
                self[holder] = Node::Empty;
                self.replace_child(grand, holder, child);
                let at = self[grand]
                    .children()
                    .iter()
                    .position(|&c| c == child)
                    .expect("a child of its parent");
                self.join(grand, at);
            }
        }
    }

    /// If the child at `at` of the linear node `parent` is a linear node of the same direction,
    /// puts its children in its place, sharing its space in the same proportions.
    fn join(&mut self, parent: NodeIndex, at: usize) {
        let child = self[parent].children()[at];
        let (Node::Linear(outer), Node::Linear(inner)) = (&self[parent], &self[child]) else {
            return;
        };
        if outer.dir != inner.dir {
            return;
        }
        let mut inner = inner.clone();
        inner.normalize_shares();
        self[child] = Node::Empty;
        let Node::Linear(outer) = &mut self[parent] else {
            unreachable!()
        };
        let share = outer.shares[at];
        outer
            .children
            .splice(at..=at, inner.children.iter().copied());
        outer
            .shares
            .splice(at..=at, inner.shares.iter().map(|s| s * share));
    }

    /// Restores the tree's shape after nodes were emptied or loaded: drops the
    /// [`Empty`](Node::Empty) children and the references to missing nodes, removes an empty
    /// container, replaces a container with one child by that child, fits a grid's shares, joins a linear node into a parent of the same
    /// direction, frees the slots no longer reached from the root, and drops the focus if it
    /// is no longer on a leaf.
    pub(crate) fn simplify(&mut self) {
        let mut seen = HashSet::new();
        match self.simplify_node(NodeIndex::root(), &mut seen) {
            None => {
                self.nodes.clear();
                self.focused_node = None;
                return;
            }
            Some(root) if root != NodeIndex::root() => self.move_node(root, NodeIndex::root()),
            Some(_) => {}
        }
        let reached: HashSet<_> = self.breadth_first_index_iter().collect();
        for (index, node) in self.nodes.iter_mut().enumerate() {
            if !reached.contains(&NodeIndex(index)) {
                *node = Node::Empty;
            }
        }
        self.trim();
        if self
            .focused_node
            .is_some_and(|f| self.nodes.get(f.0).is_none_or(|n| !n.is_leaf()))
        {
            self.focused_node = None;
        }
    }

    /// Simplifies the subtree at `index`; returns what takes its place, if anything.
    fn simplify_node(
        &mut self,
        index: NodeIndex,
        seen: &mut HashSet<NodeIndex>,
    ) -> Option<NodeIndex> {
        if !seen.insert(index) {
            return None;
        }
        match self.nodes.get(index.0)? {
            Node::Empty => None,
            Node::Leaf(_) => Some(index),
            Node::Linear(linear) => {
                let old: Vec<_> = linear
                    .children
                    .iter()
                    .copied()
                    .zip(linear.shares.iter().copied())
                    .collect();
                let mut kept = Vec::new();
                for (child, share) in old {
                    if let Some(child) = self.simplify_node(child, seen) {
                        let share = if share.is_finite() {
                            share.max(0.0)
                        } else {
                            0.0
                        };
                        kept.push((child, share));
                    }
                }
                match kept.len() {
                    0 => {
                        self[index] = Node::Empty;
                        None
                    }
                    1 => {
                        self[index] = Node::Empty;
                        Some(kept[0].0)
                    }
                    _ => {
                        let Node::Linear(linear) = &mut self[index] else {
                            unreachable!()
                        };
                        (linear.children, linear.shares) = kept.into_iter().unzip();
                        let mut at = 0;
                        while at < self[index].children().len() {
                            let before = self[index].children().len();
                            self.join(index, at);
                            at += 1 + self[index].children().len() - before;
                        }
                        Some(index)
                    }
                }
            }
            Node::Grid(grid) => {
                let old = grid.children.clone();
                let kept: Vec<_> = old
                    .into_iter()
                    .filter_map(|child| self.simplify_node(child, seen))
                    .collect();
                match kept.len() {
                    0 => {
                        self[index] = Node::Empty;
                        None
                    }
                    1 => {
                        self[index] = Node::Empty;
                        Some(kept[0])
                    }
                    _ => {
                        let Node::Grid(grid) = &mut self[index] else {
                            unreachable!()
                        };
                        grid.children = kept;
                        grid.fit_shares();
                        Some(index)
                    }
                }
            }
        }
    }

    /// Pushes a tab to the first `Leaf` it finds or creates a new leaf if the tree is empty.
    pub fn push_to_first_leaf(&mut self, tab: Tab) {
        if let Some(index) = self.first_leaf(NodeIndex::root()) {
            let Node::Leaf(leaf) = &mut self[index] else {
                unreachable!()
            };
            leaf.active = TabIndex(leaf.tabs.len());
            leaf.tabs.push(tab);
            self.focused_node = Some(index);
            return;
        }
        self.nodes.clear();
        self.nodes.push(Node::leaf_with(vec![tab]));
        self.focused_node = Some(NodeIndex(0));
    }

    /// Sets which is the active tab within a specific node.
    ///
    /// # Errors
    /// If the node is invalid, not a leaf or if the tab index is out of bounds.
    #[inline]
    pub fn set_active_tab(
        &mut self,
        node_index: impl Into<NodeIndex>,
        tab_index: impl Into<TabIndex>,
    ) -> Result {
        self.leaf_mut(node_index.into())?
            .set_active_tab(tab_index.into())
    }

    /// Pushes `tab` to the currently focused leaf.
    ///
    /// If no leaf is focused it will be pushed to the first available leaf.
    ///
    /// If no leaf is available then a new leaf will be created.
    pub fn push_to_focused_leaf(&mut self, tab: Tab) {
        match self
            .focused_node
            .and_then(|node| self.nodes.get_mut(node.0))
        {
            Some(Node::Leaf(leaf)) => leaf.append_tab(tab),
            _ => self.push_to_first_leaf(tab),
        }
    }

    /// Removes the tab at the given ([`NodeIndex`], [`TabIndex`]) pair.
    ///
    /// If the node is emptied after the tab is removed, the node will also be removed.
    ///
    /// Returns the removed tab if it exists, or `None` otherwise.
    pub fn remove_tab(&mut self, (node_index, tab_index): (NodeIndex, TabIndex)) -> Option<Tab> {
        let node = &mut self[node_index];
        let tab = node.remove_tab(tab_index);
        if node.tabs_count() == 0 {
            self.remove_leaf(node_index);
        }
        tab
    }

    /// Returns a new [`Tree`] while mapping and filtering the tab type.
    /// Any remaining empty [`Node`]s are removed.
    pub fn filter_map_tabs<F, NewTab>(&self, mut function: F) -> Tree<NewTab>
    where
        F: FnMut(&Tab) -> Option<NewTab>,
    {
        let Tree {
            focused_node,
            nodes,
            collapsed,
            collapsed_leaf_count,
        } = self;
        let nodes = nodes
            .iter()
            .map(|node| node.filter_map_tabs(&mut function))
            .collect();
        let mut new_tree = Tree {
            nodes,
            focused_node: *focused_node,
            collapsed: *collapsed,
            collapsed_leaf_count: *collapsed_leaf_count,
        };
        new_tree.simplify();
        new_tree
    }

    /// Returns a new [`Tree`] while mapping the tab type.
    pub fn map_tabs<F, NewTab>(&self, mut function: F) -> Tree<NewTab>
    where
        F: FnMut(&Tab) -> NewTab,
    {
        self.filter_map_tabs(move |tab| Some(function(tab)))
    }

    /// Returns a new [`Tree`] while filtering the tab type.
    /// Any remaining empty [`Node`]s are removed.
    pub fn filter_tabs<F>(&self, mut predicate: F) -> Tree<Tab>
    where
        F: FnMut(&Tab) -> bool,
        Tab: Clone,
    {
        self.filter_map_tabs(move |tab| predicate(tab).then(|| tab.clone()))
    }

    /// Removes all tabs for which `predicate` returns `false`.
    /// Any remaining empty [`Node`]s are also removed.
    pub fn retain_tabs<F>(&mut self, mut predicate: F)
    where
        F: FnMut(&mut Tab) -> bool,
    {
        for node in self.nodes.iter_mut() {
            node.retain_tabs(&mut predicate);
        }
        self.simplify();
    }

    /// Sets the collapsing state of the [`Tree`].
    pub(crate) fn set_collapsed(&mut self, collapsed: bool) {
        self.collapsed = collapsed;
    }

    /// Returns whether the [`Tree`] is collapsed.
    pub(crate) fn is_collapsed(&self) -> bool {
        self.collapsed
    }

    /// Sets the number of collapsed layers of leaf subnodes in the [`Tree`].
    pub(crate) fn set_collapsed_leaf_count(&mut self, collapsed_leaf_count: i32) {
        self.collapsed_leaf_count = collapsed_leaf_count;
    }

    /// Updates the collapsed state of the node and its parents.
    pub(crate) fn node_update_collapsed(&mut self, node_index: NodeIndex) {
        let collapsed = self[node_index].is_collapsed();
        let mut ancestor = self.parent(node_index);
        while let Some(index) = ancestor {
            ancestor = self.parent(index);
            let all_collapsed = self.update_collapsed_leaf_count(index);
            if !collapsed {
                self[index].set_collapsed(false);
            } else if all_collapsed {
                self[index].set_collapsed(true);
            }
        }
        let root = self
            .root_node()
            .map(|r| (r.is_collapsed(), r.collapsed_leaf_count()));
        if !collapsed {
            self.set_collapsed(false);
            if let Some((_, count)) = root {
                self.set_collapsed_leaf_count(count);
            }
        } else if let Some((true, count)) = root {
            self.set_collapsed(true);
            self.set_collapsed_leaf_count(count);
        }
    }

    /// Recounts the collapsed leaves of the container `index` from its children; returns
    /// whether all its children are collapsed.
    fn update_collapsed_leaf_count(&mut self, index: NodeIndex) -> bool {
        let counts: Vec<_> = self[index]
            .children()
            .iter()
            .map(|&c| (self[c].is_collapsed(), self[c].collapsed_leaf_count()))
            .collect();
        let all_collapsed = counts.iter().all(|&(collapsed, _)| collapsed);
        let counts: Vec<i32> = counts.iter().map(|&(_, count)| count).collect();
        let count = match &self[index] {
            Node::Linear(LinearNode {
                dir: LinearDir::Horizontal,
                ..
            }) => counts.iter().copied().max().unwrap_or(0),
            Node::Linear(_) => counts.iter().sum(),
            // Each row counts as its tallest child.
            Node::Grid(grid) => counts
                .chunks(grid.columns())
                .map(|row| row.iter().copied().max().unwrap_or(0))
                .sum(),
            _ => unreachable!("a parent is a container"),
        };
        self[index].set_collapsed_leaf_count(count);
        all_collapsed
    }

    /// Lays out the children of the container `node` in a [`Grid`](Node::Grid), in their
    /// order, with even columns and rows: a row or a column becomes a grid, and a grid takes
    /// the new `layout`. The children keep their indices.
    ///
    /// Returns `false`, changing nothing, if `node` is not a [`Linear`](Node::Linear) or a
    /// [`Grid`](Node::Grid) node.
    ///
    /// # Example
    ///
    /// ```rust
    /// # use egui_stack::{DockState, GridLayout, NodeIndex, Node};
    /// let mut dock_state = DockState::new(vec!["a"]);
    /// let tree = dock_state.main_surface_mut();
    /// let [a, b] = tree.split_right(NodeIndex::root(), 0.5, vec!["b"]);
    /// let [_, c] = tree.split_right(b, 0.5, vec!["c"]);
    /// let [_, d] = tree.split_right(c, 0.5, vec!["d"]);
    ///
    /// assert!(tree.arrange_as_grid(NodeIndex::root(), GridLayout::Auto));
    /// let grid = tree.root_node().unwrap().get_grid().unwrap();
    /// assert_eq!(grid.children(), &[a, b, c, d]);
    /// assert_eq!((grid.columns(), grid.rows()), (2, 2));
    /// ```
    pub fn arrange_as_grid(&mut self, node: NodeIndex, layout: GridLayout) -> bool {
        let grid = match self.nodes.get(node.0) {
            Some(Node::Linear(linear)) => {
                let mut grid = GridNode::new(layout, linear.children.clone());
                grid.rect = linear.rect;
                grid.fully_collapsed = linear.fully_collapsed;
                grid
            }
            Some(Node::Grid(grid)) => {
                let mut grid = GridNode::new(layout, grid.children.clone());
                grid.rect = self[node].rect().unwrap_or(Rect::NOTHING);
                grid.fully_collapsed = self[node].is_collapsed();
                grid
            }
            _ => return false,
        };
        self[node] = Node::Grid(grid);
        self.update_collapsed_leaf_count(node);
        self.node_update_collapsed(node);
        true
    }

    /// Lays out the children of the container `node` in a [`Linear`](Node::Linear) node of
    /// direction `dir`, in their order: a grid becomes a row or a column (with even shares,
    /// or its column shares if it has one row, its row shares if it has one column), and a
    /// row becomes a column or the other way round. The children keep their indices.
    ///
    /// The tree stays flat: children that are linear nodes of direction `dir` join the new
    /// one, and it joins its parent if that is a linear node of direction `dir`. Joined
    /// containers are removed, so `node` itself may be gone; the root stays at the root.
    ///
    /// Returns `false`, changing nothing, if `node` is not a [`Linear`](Node::Linear) or a
    /// [`Grid`](Node::Grid) node.
    pub fn arrange_as_linear(&mut self, node: NodeIndex, dir: LinearDir) -> bool {
        let linear = match self.nodes.get(node.0) {
            Some(Node::Linear(linear)) => {
                let mut linear = linear.clone();
                linear.dir = dir;
                linear
            }
            Some(Node::Grid(grid)) => {
                let shares = match dir {
                    LinearDir::Horizontal if grid.rows() == 1 => grid.col_shares.clone(),
                    LinearDir::Vertical if grid.columns() == 1 => grid.row_shares.clone(),
                    _ => vec![1.0; grid.len()],
                };
                let mut linear = LinearNode::new(dir, grid.children.clone(), shares);
                linear.rect = grid.rect;
                linear.fully_collapsed = grid.fully_collapsed;
                linear
            }
            _ => return false,
        };
        self[node] = Node::Linear(linear);
        let mut at = 0;
        while at < self[node].children().len() {
            let before = self[node].children().len();
            self.join(node, at);
            at += 1 + self[node].children().len() - before;
        }
        self.update_collapsed_leaf_count(node);
        self.node_update_collapsed(node);
        if let Some(parent) = self.parent(node) {
            let at = self[parent]
                .children()
                .iter()
                .position(|&c| c == node)
                .expect("a child of its parent");
            self.join(parent, at);
            self.update_collapsed_leaf_count(parent);
            self.trim();
        }
        true
    }

    /// Find a given tab based on ``predicate``.
    ///
    /// Returns the indices in where that node and tab is in this surface.
    ///
    /// The returned [`NodeIndex`] will always point to a [`Node::Leaf`].
    ///
    /// In case there are several hits, only the first is returned.
    pub fn find_tab_from(&self, predicate: impl Fn(&Tab) -> bool) -> Option<(NodeIndex, TabIndex)> {
        for (node_index, node) in self.nodes.iter().enumerate() {
            if let Some(tabs) = node.tabs() {
                for (tab_index, tab) in tabs.iter().enumerate() {
                    if predicate(tab) {
                        return Some((node_index.into(), tab_index.into()));
                    }
                }
            };
        }
        None
    }
}

impl<Tab> Tree<Tab>
where
    Tab: PartialEq,
{
    /// Find the given tab.
    ///
    /// Returns in which node and where in that node the tab is.
    ///
    /// The returned [`NodeIndex`] will always point to a [`Node::Leaf`].
    ///
    /// In case there are several hits, only the first is returned.
    pub fn find_tab(&self, needle_tab: &Tab) -> Option<(NodeIndex, TabIndex)> {
        self.find_tab_from(|tab| tab == needle_tab)
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[derive(Copy, Clone, Debug, PartialEq)]
    struct Tab(u64);

    /// Checks that `retain` works after removing a node
    #[test]
    fn remove_and_retain() {
        let mut tree: Tree<Tab> = Tree::new(vec![]);
        tree.push_to_focused_leaf(Tab(0));
        let (n0, _t0) = tree.find_tab(&Tab(0)).unwrap();
        tree.split_below(n0, 0.5, vec![Tab(1)]);

        let i1 = tree.find_tab(&Tab(1)).unwrap();
        tree.remove_tab(i1);
        assert_eq!(tree.nodes.len(), 1);

        tree.retain_tabs(|_| true);
        assert!(tree.find_tab(&Tab(0)).is_some());
    }

    /// Tests whether `retain_tabs` works correctly with trailing `Empty` nodes
    #[test]
    fn retain_trailing_empty() {
        let mut tree: Tree<Tab> = Tree::new(vec![]);
        tree.push_to_focused_leaf(Tab(0));
        tree.nodes.push(Node::Empty);
        tree.nodes.push(Node::Empty);

        tree.retain_tabs(|_| true);
        assert!(tree.find_tab(&Tab(0)).is_some());
    }

    fn linear(tree: &Tree<Tab>, index: NodeIndex) -> &LinearNode {
        tree[index].get_linear().expect("a linear node")
    }

    fn close(a: &[f32], b: &[f32]) -> bool {
        a.len() == b.len() && a.iter().zip(b).all(|(a, b)| (a - b).abs() < 1e-5)
    }

    /// Three leaves in a row: `[0 | 1 | 2]` with the shares `[0.5, 0.25, 0.25]`.
    fn row() -> (Tree<Tab>, [NodeIndex; 3]) {
        let mut tree = Tree::new(vec![Tab(0)]);
        let [a, b] = tree.split_right(NodeIndex::root(), 0.5, vec![Tab(1)]);
        let [b2, c] = tree.split_right(b, 0.5, vec![Tab(2)]);
        assert_eq!(b, b2, "a split node keeps its index");
        (tree, [a, b, c])
    }

    #[test]
    fn a_split_along_its_parent_joins_the_row() {
        let (tree, [a, b, c]) = row();
        let root = linear(&tree, NodeIndex::root());
        assert_eq!(root.dir, LinearDir::Horizontal);
        assert_eq!(root.children(), &[a, b, c]);
        assert!(close(root.shares(), &[0.5, 0.25, 0.25]));
        assert_eq!(tree.focused_leaf(), Some(c));
    }

    #[test]
    fn a_split_before_takes_the_first_share() {
        let (mut tree, [a, b, c]) = row();
        let [b2, d] = tree.split_left(b, 0.2, vec![Tab(3)]);
        assert_eq!(b2, b);
        let root = linear(&tree, NodeIndex::root());
        assert_eq!(root.children(), &[a, d, b, c]);
        assert!(close(root.shares(), &[0.5, 0.05, 0.2, 0.25]));
    }

    #[test]
    fn a_split_across_its_parent_nests() {
        let (mut tree, [a, b, c]) = row();
        let [b2, d] = tree.split_below(b, 0.5, vec![Tab(3)]);
        assert_eq!(b2, b);
        let root = linear(&tree, NodeIndex::root());
        assert_eq!(root.children()[0], a);
        assert_eq!(root.children()[2], c);
        let column = root.children()[1];
        assert_eq!(tree.parent(b), Some(column));
        let column = linear(&tree, column);
        assert_eq!(column.dir, LinearDir::Vertical);
        assert_eq!(column.children(), &[b, d]);
    }

    #[test]
    fn removing_a_leaf_keeps_the_others_in_proportion() {
        let (mut tree, [a, b, c]) = row();
        tree.remove_leaf(a);
        let root = linear(&tree, NodeIndex::root());
        assert_eq!(root.children(), &[b, c]);
        assert_eq!(root.fraction(0), 0.5);
        // The focus was on `c` and stays.
        assert_eq!(tree.focused_leaf(), Some(c));

        tree.remove_leaf(c);
        assert!(tree[NodeIndex::root()].is_leaf());
        assert_eq!(tree[NodeIndex::root()].tabs(), Some([Tab(1)].as_slice()));
        assert_eq!(tree.focused_leaf(), Some(NodeIndex::root()));
        assert_eq!(tree.len(), 1);
    }

    #[test]
    fn a_column_left_with_one_child_gives_way_and_joins_the_row() {
        // [0 | (1 / [2 | 3]) ] : removing 1 leaves [0 | 2 | 3].
        let mut tree = Tree::new(vec![Tab(0)]);
        let [a, b] = tree.split_right(NodeIndex::root(), 0.5, vec![Tab(1)]);
        let [_, c] = tree.split_below(b, 0.5, vec![Tab(2)]);
        let [_, d] = tree.split_right(c, 0.5, vec![Tab(3)]);
        tree.remove_leaf(b);
        let root = linear(&tree, NodeIndex::root());
        assert_eq!(root.children(), &[a, c, d]);
        assert!(close(root.shares(), &[0.5, 0.25, 0.25]));
        assert_eq!(tree.breadth_first_index_iter().count(), 4);
    }

    #[test]
    fn removing_the_focused_leaf_focuses_a_neighbour() {
        let (mut tree, [_, b, c]) = row();
        tree.set_focused_node(c);
        tree.remove_leaf(c);
        assert_eq!(tree.focused_leaf(), Some(b));
    }

    #[test]
    fn retain_simplifies_the_tree() {
        let (mut tree, [a, _, c]) = row();
        tree.retain_tabs(|tab| tab.0 != 1);
        let root = linear(&tree, NodeIndex::root());
        assert_eq!(root.children(), &[a, c]);
        assert!(close(root.shares(), &[0.5, 0.25]));

        tree.retain_tabs(|tab| tab.0 == 2);
        assert_eq!(tree[NodeIndex::root()].tabs(), Some([Tab(2)].as_slice()));
        assert_eq!(tree.len(), 1);

        tree.retain_tabs(|_| false);
        assert!(tree.is_empty());
    }

    #[test]
    fn insert_child_gives_an_even_share() {
        let (mut tree, [a, b, c]) = row();
        let d = tree.insert_child(NodeIndex::root(), 0, Node::leaf(Tab(3)));
        let root = linear(&tree, NodeIndex::root());
        assert_eq!(root.children(), &[d, a, b, c]);
        assert!(close(root.shares(), &[0.25, 0.375, 0.1875, 0.1875]));
    }

    #[test]
    fn freed_slots_are_reused() {
        let (mut tree, [a, ..]) = row();
        let len = tree.len();
        tree.remove_leaf(a);
        let [_, d] = tree.split_left(NodeIndex(2), 0.5, vec![Tab(3)]);
        assert_eq!(d, a);
        assert_eq!(tree.len(), len);
    }

    #[test]
    fn push_to_first_leaf_in_layout_order() {
        let (mut tree, [a, ..]) = row();
        tree.push_to_first_leaf(Tab(9));
        assert_eq!(tree[a].tabs(), Some([Tab(0), Tab(9)].as_slice()));
    }

    fn grid(tree: &Tree<Tab>, index: NodeIndex) -> &GridNode {
        tree[index].get_grid().expect("a grid node")
    }

    /// Four leaves in a row turned into a 2 by 2 grid.
    fn grid_of_four() -> (Tree<Tab>, [NodeIndex; 4]) {
        let (mut tree, [a, b, c]) = row();
        let [_, d] = tree.split_right(c, 0.5, vec![Tab(3)]);
        assert!(tree.arrange_as_grid(NodeIndex::root(), GridLayout::Auto));
        (tree, [a, b, c, d])
    }

    #[test]
    fn a_row_arranged_as_a_grid_keeps_its_children() {
        let (tree, [a, b, c, d]) = grid_of_four();
        let grid = grid(&tree, NodeIndex::root());
        assert_eq!(grid.children(), &[a, b, c, d]);
        assert_eq!((grid.columns(), grid.rows()), (2, 2));
        assert!(close(grid.col_shares(), &[1.0, 1.0]));
        assert!(close(grid.row_shares(), &[1.0, 1.0]));
        assert_eq!(tree.parent(c), Some(NodeIndex::root()));
    }

    #[test]
    fn only_containers_are_arranged() {
        let mut tree = Tree::new(vec![Tab(0)]);
        assert!(!tree.arrange_as_grid(NodeIndex::root(), GridLayout::Auto));
        assert!(!tree.arrange_as_linear(NodeIndex::root(), LinearDir::Vertical));
        assert!(tree[NodeIndex::root()].is_leaf());
    }

    #[test]
    fn a_split_in_a_grid_adds_a_child_next_to_the_leaf() {
        let (mut tree, [a, b, c, d]) = grid_of_four();
        let [b2, e] = tree.split_below(b, 0.5, vec![Tab(4)]);
        assert_eq!(b2, b);
        let grid = grid(&tree, NodeIndex::root());
        assert_eq!(grid.children(), &[a, b, e, c, d]);
        assert_eq!((grid.columns(), grid.rows()), (3, 2));
        assert_eq!(grid.col_shares().len(), 3);

        let [_, f] = tree.split_left(a, 0.5, vec![Tab(5)]);
        assert_eq!(grid_children(&tree), vec![f, a, b, e, c, d]);
        assert_eq!(tree.focused_leaf(), Some(f));
    }

    fn grid_children(tree: &Tree<Tab>) -> Vec<NodeIndex> {
        grid(tree, NodeIndex::root()).children().to_vec()
    }

    #[test]
    fn insert_child_adds_to_a_grid() {
        let (mut tree, [a, b, c, d]) = grid_of_four();
        let e = tree.insert_child(NodeIndex::root(), 4, Node::leaf(Tab(4)));
        assert_eq!(grid_children(&tree), vec![a, b, c, d, e]);
        let grid = grid(&tree, NodeIndex::root());
        assert_eq!((grid.columns(), grid.rows()), (3, 2));
        assert_eq!(tree.focused_leaf(), Some(e));
    }

    #[test]
    fn a_grid_left_with_one_child_gives_way() {
        // [x | grid(a, b, c, d)]: emptying the grid down to `d` leaves [x | d].
        let mut tree = Tree::new(vec![Tab(9)]);
        let [x, a] = tree.split_right(NodeIndex::root(), 0.5, vec![Tab(0)]);
        let [_, b] = tree.split_below(a, 0.5, vec![Tab(1)]);
        let column = tree.parent(a).unwrap();
        let [_, c] = tree.split_below(b, 0.5, vec![Tab(2)]);
        let [_, d] = tree.split_below(c, 0.5, vec![Tab(3)]);
        assert!(tree.arrange_as_grid(column, GridLayout::Auto));
        tree.remove_leaf(a);
        tree.remove_leaf(b);
        let grid = grid(&tree, column);
        assert_eq!(grid.children(), &[c, d]);
        assert_eq!((grid.columns(), grid.rows()), (2, 1));
        tree.remove_leaf(c);
        let root = linear(&tree, NodeIndex::root());
        assert_eq!(root.children(), &[x, d]);
        assert!(tree[column].is_empty() || column.0 >= tree.len());
    }

    #[test]
    fn retain_removes_an_emptied_grid() {
        // [x | grid(a, b, c)]
        let mut tree = Tree::new(vec![Tab(9)]);
        let [x, a] = tree.split_right(NodeIndex::root(), 0.5, vec![Tab(0)]);
        let [_, b] = tree.split_below(a, 0.5, vec![Tab(1)]);
        tree.split_below(b, 0.5, vec![Tab(2)]);
        let column = tree.parent(a).unwrap();
        assert!(tree.arrange_as_grid(column, GridLayout::Columns(2)));

        tree.retain_tabs(|tab| tab.0 != 1);
        assert_eq!(grid(&tree, column).len(), 2);
        assert_eq!(grid(&tree, column).rows(), 1);

        tree.retain_tabs(|tab| tab.0 == 9);
        assert_eq!(tree.len(), 1);
        assert_eq!(tree[NodeIndex::root()].tabs(), Some([Tab(9)].as_slice()));
        let _ = x;
    }

    #[test]
    fn a_grid_arranged_as_a_column_joins_its_parent() {
        // (x / grid(a, b, c, d)) as a column gives (x / a / b / c / d).
        let mut tree = Tree::new(vec![Tab(9)]);
        let [x, a] = tree.split_below(NodeIndex::root(), 0.5, vec![Tab(0)]);
        let [_, b] = tree.split_right(a, 0.5, vec![Tab(1)]);
        let [_, c] = tree.split_right(b, 0.5, vec![Tab(2)]);
        let [_, d] = tree.split_right(c, 0.5, vec![Tab(3)]);
        let row = tree.parent(a).unwrap();
        assert!(tree.arrange_as_grid(row, GridLayout::Auto));
        assert!(tree.arrange_as_linear(row, LinearDir::Vertical));
        let root = linear(&tree, NodeIndex::root());
        assert_eq!(root.dir, LinearDir::Vertical);
        assert_eq!(root.children(), &[x, a, b, c, d]);
        assert!(close(root.shares(), &[0.5, 0.125, 0.125, 0.125, 0.125]));
    }

    #[test]
    fn a_grid_of_one_row_keeps_its_column_shares_as_a_row() {
        let (mut tree, _) = row();
        assert!(tree.arrange_as_grid(NodeIndex::root(), GridLayout::Columns(3)));
        tree[NodeIndex::root()]
            .get_grid_mut()
            .unwrap()
            .set_shares(vec![2.0, 1.0, 1.0], vec![1.0]);
        assert!(tree.arrange_as_linear(NodeIndex::root(), LinearDir::Horizontal));
        assert!(close(
            linear(&tree, NodeIndex::root()).shares(),
            &[2.0, 1.0, 1.0]
        ));
    }

    #[test]
    fn a_grid_counts_collapsed_leaves_per_row() {
        let (mut tree, [a, b, c, d]) = grid_of_four();
        for leaf in [a, b, c, d] {
            tree[leaf].set_collapsed(true);
            tree.node_update_collapsed(leaf);
        }
        assert!(tree[NodeIndex::root()].is_collapsed());
        assert_eq!(tree[NodeIndex::root()].collapsed_leaf_count(), 2);
    }
}
