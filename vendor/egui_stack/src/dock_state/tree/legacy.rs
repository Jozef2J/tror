//! Loading trees saved by earlier versions.
//!
//! Up to 0.23, a [`Tree`] was a binary tree stored as a heap: the children of the node at
//! index *n* were at *2n + 1* and *2n + 2*, and a parent was a `Horizontal` or a `Vertical`
//! split with the `fraction` of its first child. Such a tree loads as linear nodes with the
//! same layout: each split becomes a linear node of two children with the shares
//! `[fraction, 1 - fraction]`, keeping the indices, then nested splits of the same direction
//! are joined into one row or column.

use egui::Rect;

use super::{GridNode, LeafNode, LinearDir, LinearNode, Node, NodeIndex, Tree};

/// A node as saved by this version or an earlier one.
#[derive(serde::Deserialize)]
pub(super) enum NodeRepr<Tab> {
    Empty,
    Leaf(LeafNode<Tab>),
    Linear(LinearNode),
    Grid(GridNode),
    Vertical(SplitRepr),
    Horizontal(SplitRepr),
}

/// A binary split as saved up to 0.23.
#[derive(serde::Deserialize)]
#[serde(default)]
pub(super) struct SplitRepr {
    rect: Rect,
    fraction: f32,
    fully_collapsed: bool,
    collapsed_leaf_count: i32,
}

impl Default for SplitRepr {
    fn default() -> Self {
        Self {
            rect: Rect::NOTHING,
            fraction: 0.5,
            fully_collapsed: false,
            collapsed_leaf_count: 0,
        }
    }
}

/// A tree as saved by this version or an earlier one.
#[derive(serde::Deserialize)]
#[serde(default, bound(deserialize = "Tab: serde::Deserialize<'de>"))]
pub(super) struct TreeRepr<Tab> {
    nodes: Vec<NodeRepr<Tab>>,
    focused_node: Option<NodeIndex>,
    collapsed: bool,
    collapsed_leaf_count: i32,
}

impl<Tab> Default for TreeRepr<Tab> {
    fn default() -> Self {
        Self {
            nodes: Vec::new(),
            focused_node: None,
            collapsed: false,
            collapsed_leaf_count: 0,
        }
    }
}

impl<Tab> From<TreeRepr<Tab>> for Tree<Tab> {
    fn from(repr: TreeRepr<Tab>) -> Self {
        let nodes = repr
            .nodes
            .into_iter()
            .enumerate()
            .map(|(index, node)| match node {
                NodeRepr::Empty => Node::Empty,
                NodeRepr::Leaf(leaf) => Node::Leaf(leaf),
                NodeRepr::Linear(linear) => Node::Linear(linear),
                NodeRepr::Grid(grid) => Node::Grid(grid),
                NodeRepr::Horizontal(split) => split.into_linear(LinearDir::Horizontal, index),
                NodeRepr::Vertical(split) => split.into_linear(LinearDir::Vertical, index),
            })
            .collect();
        let mut tree = Tree {
            nodes,
            focused_node: repr.focused_node,
            collapsed: repr.collapsed,
            collapsed_leaf_count: repr.collapsed_leaf_count,
        };
        // A saved tree is checked: no dangling child, no linear node of one child. Nested
        // splits of one direction, all an old tree could hold, join into rows and columns.
        tree.simplify();
        tree
    }
}

impl SplitRepr {
    fn into_linear<Tab>(self, dir: LinearDir, index: usize) -> Node<Tab> {
        let fraction = if self.fraction.is_finite() {
            self.fraction.clamp(0.0, 1.0)
        } else {
            0.5
        };
        let mut linear = LinearNode::new(
            dir,
            vec![NodeIndex(index * 2 + 1), NodeIndex(index * 2 + 2)],
            vec![fraction, 1.0 - fraction],
        );
        linear.rect = self.rect;
        linear.fully_collapsed = self.fully_collapsed;
        linear.collapsed_leaf_count = self.collapsed_leaf_count;
        Node::Linear(linear)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leaf(tab: &str) -> String {
        format!(r#"{{"Leaf":{{"tabs":["{tab}"]}}}}"#)
    }

    fn split(dir: &str, fraction: f32) -> String {
        format!(
            r#"{{"{dir}":{{"fraction":{fraction},"fully_collapsed":false,"collapsed_leaf_count":0}}}}"#
        )
    }

    /// `a | (b | c)` as saved up to 0.23: a heap of two horizontal splits.
    fn three_columns() -> String {
        let nodes = [
            split("Horizontal", 0.25),
            leaf("a"),
            split("Horizontal", 0.5),
            r#""Empty""#.to_owned(),
            r#""Empty""#.to_owned(),
            leaf("b"),
            leaf("c"),
        ];
        format!(
            r#"{{"nodes":[{}],"focused_node":5,"collapsed":false,"collapsed_leaf_count":0}}"#,
            nodes.join(",")
        )
    }

    #[test]
    fn nested_splits_of_one_direction_load_as_one_row() {
        let tree: Tree<String> = serde_json::from_str(&three_columns()).unwrap();
        let root = tree[NodeIndex::root()].get_linear().unwrap();
        assert_eq!(root.dir, LinearDir::Horizontal);
        assert_eq!(root.children(), &[NodeIndex(1), NodeIndex(5), NodeIndex(6)]);
        let expected = [0.25, 0.375, 0.375];
        for (share, expected) in root.shares().iter().zip(expected) {
            assert!((share - expected).abs() < 1e-6, "{:?}", root.shares());
        }
        // Leaves keep their indices, so the focus does too.
        assert_eq!(tree.focused_leaf(), Some(NodeIndex(5)));
        assert_eq!(tree[NodeIndex(5)].tabs(), Some(["b".to_owned()].as_slice()));
        // The joined split's slot is free.
        assert!(tree[NodeIndex(2)].is_empty());
    }

    #[test]
    fn splits_of_both_directions_load_nested() {
        // a | (b / c)
        let nodes = [
            split("Horizontal", 0.5),
            leaf("a"),
            split("Vertical", 0.3),
            r#""Empty""#.to_owned(),
            r#""Empty""#.to_owned(),
            leaf("b"),
            leaf("c"),
        ];
        let json = format!(r#"{{"nodes":[{}]}}"#, nodes.join(","));
        let tree: Tree<String> = serde_json::from_str(&json).unwrap();
        let root = tree[NodeIndex::root()].get_linear().unwrap();
        assert_eq!(root.children(), &[NodeIndex(1), NodeIndex(2)]);
        let column = tree[NodeIndex(2)].get_linear().unwrap();
        assert_eq!(column.dir, LinearDir::Vertical);
        assert_eq!(column.children(), &[NodeIndex(5), NodeIndex(6)]);
        assert!((column.fraction(0) - 0.3).abs() < 1e-6);
    }

    #[test]
    fn a_split_with_a_missing_child_gives_way() {
        let json = format!(r#"{{"nodes":[{},{}]}}"#, split("Vertical", 0.5), leaf("a"));
        let tree: Tree<String> = serde_json::from_str(&json).unwrap();
        assert_eq!(tree.len(), 1);
        assert_eq!(
            tree[NodeIndex::root()].tabs(),
            Some(["a".to_owned()].as_slice())
        );
    }

    #[test]
    fn a_tree_saved_today_loads_unchanged() {
        let mut tree = Tree::new(vec!["a".to_owned()]);
        let [_, b] = tree.split_right(NodeIndex::root(), 0.5, vec!["b".to_owned()]);
        tree.split_right(b, 0.25, vec!["c".to_owned()]);
        // Laid out, as a saved tree is: JSON has no infinite numbers for `Rect::NOTHING`.
        for node in tree.iter_mut() {
            node.set_rect(Rect::ZERO);
            if let Some(leaf) = node.get_leaf_mut() {
                leaf.viewport = Rect::ZERO;
            }
        }
        let json = serde_json::to_string(&tree).unwrap();
        assert!(json.contains("Linear"), "{json}");
        let loaded: Tree<String> = serde_json::from_str(&json).unwrap();
        assert_eq!(serde_json::to_string(&loaded).unwrap(), json);
    }

    #[test]
    fn a_grid_round_trips() {
        let mut tree = Tree::new(vec!["a".to_owned()]);
        let [_, b] = tree.split_right(NodeIndex::root(), 0.5, vec!["b".to_owned()]);
        tree.split_right(b, 0.5, vec!["c".to_owned()]);
        assert!(tree.arrange_as_grid(NodeIndex::root(), crate::GridLayout::Columns(2)));
        tree[NodeIndex::root()]
            .get_grid_mut()
            .unwrap()
            .set_shares(vec![1.0, 2.0], vec![3.0, 1.0]);
        for node in tree.iter_mut() {
            node.set_rect(Rect::ZERO);
            if let Some(leaf) = node.get_leaf_mut() {
                leaf.viewport = Rect::ZERO;
            }
        }
        let json = serde_json::to_string(&tree).unwrap();
        assert!(json.contains("Grid"), "{json}");
        let loaded: Tree<String> = serde_json::from_str(&json).unwrap();
        assert_eq!(serde_json::to_string(&loaded).unwrap(), json);
        let grid = loaded[NodeIndex::root()].get_grid().unwrap();
        assert_eq!(grid.layout, crate::GridLayout::Columns(2));
        assert_eq!(grid.col_shares(), &[1.0, 2.0]);
    }

    #[test]
    fn a_saved_grid_is_checked() {
        // A grid of one live child and one missing gives way; shares of the wrong length fit.
        let json = format!(
            r#"{{"nodes":[{{"Grid":{{"children":[1,2,7],"col_shares":[1.0]}}}},{},{}]}}"#,
            leaf("a"),
            leaf("b")
        );
        let tree: Tree<String> = serde_json::from_str(&json).unwrap();
        let grid = tree[NodeIndex::root()].get_grid().unwrap();
        assert_eq!(grid.children(), &[NodeIndex(1), NodeIndex(2)]);
        assert_eq!(grid.col_shares(), &[1.0, 1.0]);
        assert_eq!(grid.row_shares(), &[1.0]);

        let json = format!(
            r#"{{"nodes":[{{"Grid":{{"children":[1,5]}}}},{}]}}"#,
            leaf("a")
        );
        let tree: Tree<String> = serde_json::from_str(&json).unwrap();
        assert_eq!(tree.len(), 1);
        assert!(tree[NodeIndex::root()].is_leaf());
    }
}
