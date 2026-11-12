use egui::Rect;

use super::shares;
use crate::NodeIndex;

/// How a [`GridNode`] chooses its number of columns.
///
/// More layouts may come: the enum is `#[non_exhaustive]`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[non_exhaustive]
pub enum GridLayout {
    /// As many columns as the square root of the number of children, rounded up: 2 columns
    /// for 2 to 4 children, 3 for 5 to 9, and so on. The rows fill from the top; the last
    /// one may be short.
    ///
    /// This is the rule of egui_tiles' automatic grid (<https://github.com/rerun-io/egui_tiles>,
    /// MIT OR Apache-2.0).
    #[default]
    Auto,

    /// This many columns (at least 1, at most the number of children).
    Columns(usize),
}

/// The inner data of a [`Node::Grid`](crate::Node): children laid out in rows and columns,
/// left to right then top to bottom.
///
/// Each column and each row has a share of the space, a relative weight as in a
/// [`LinearNode`](crate::LinearNode). A separator stands on each line between two columns and
/// on each line between two rows; it resizes the two columns, or the two rows, beside it.
///
/// The number of columns follows [`layout`](Self::layout) and the number of children, so the
/// shares are fitted to the columns and rows whenever the children change: a new column or
/// row gets the mean share of the others.
///
/// [`Tree`](crate::Tree) adds and removes children; [`Tree::arrange_as_grid`](crate::Tree)
/// turns a row or a column into a grid.
#[derive(Clone, Debug)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Deserialize, serde::Serialize),
    serde(default)
)]
pub struct GridNode {
    /// The rectangle in which all children of this node are drawn.
    pub rect: Rect,

    /// How the number of columns is chosen.
    pub layout: GridLayout,

    /// The children, row by row.
    pub(crate) children: Vec<NodeIndex>,

    /// One share per column.
    pub(crate) col_shares: Vec<f32>,

    /// One share per row.
    pub(crate) row_shares: Vec<f32>,

    /// Whether all subnodes are collapsed.
    pub fully_collapsed: bool,

    /// The number of collapsed leaf subnodes.
    pub collapsed_leaf_count: i32,
}

impl Default for GridNode {
    fn default() -> Self {
        Self::new(GridLayout::default(), Vec::new())
    }
}

impl GridNode {
    /// Creates a grid holding `children`, row by row, with even columns and rows.
    pub fn new(layout: GridLayout, children: Vec<NodeIndex>) -> Self {
        let mut grid = Self {
            rect: Rect::NOTHING,
            layout,
            children,
            col_shares: Vec::new(),
            row_shares: Vec::new(),
            fully_collapsed: false,
            collapsed_leaf_count: 0,
        };
        grid.fit_shares();
        grid
    }

    /// Set the area which this node occupies.
    #[inline]
    pub fn set_rect(&mut self, new_rect: Rect) {
        self.rect = new_rect;
    }

    /// Get the area which this node occupies.
    pub fn rect(&self) -> Rect {
        self.rect
    }

    /// The children, row by row, each row from left to right.
    pub fn children(&self) -> &[NodeIndex] {
        &self.children
    }

    /// The number of children.
    pub fn len(&self) -> usize {
        self.children.len()
    }

    /// Whether the node has no children (never the case in a [`Tree`](crate::Tree)).
    pub fn is_empty(&self) -> bool {
        self.children.is_empty()
    }

    /// Where `child` is among the children.
    pub fn position(&self, child: NodeIndex) -> Option<usize> {
        self.children.iter().position(|&c| c == child)
    }

    /// The number of columns: see [`GridLayout`]. At least 1, even with no children.
    pub fn columns(&self) -> usize {
        columns(self.layout, self.children.len())
    }

    /// The number of rows: enough for all children, the last row possibly short.
    pub fn rows(&self) -> usize {
        self.children.len().div_ceil(self.columns()).max(1)
    }

    /// The column and the row of the child at `index`.
    pub fn cell(&self, index: usize) -> (usize, usize) {
        let columns = self.columns();
        (index % columns, index / columns)
    }

    /// The shares of the columns, left to right.
    pub fn col_shares(&self) -> &[f32] {
        &self.col_shares
    }

    /// The shares of the rows, top to bottom.
    pub fn row_shares(&self) -> &[f32] {
        &self.row_shares
    }

    /// The part of the width (0 to 1) the column `index` gets.
    ///
    /// # Panics
    ///
    /// If `index` is not a column.
    pub fn col_fraction(&self, index: usize) -> f32 {
        shares::fraction(&self.col_shares, index)
    }

    /// The part of the height (0 to 1) the row `index` gets.
    ///
    /// # Panics
    ///
    /// If `index` is not a row.
    pub fn row_fraction(&self, index: usize) -> f32 {
        shares::fraction(&self.row_shares, index)
    }

    /// Where the line after the column `index` stands, from 0 (the left of the node) to 1.
    pub fn col_boundary(&self, index: usize) -> f32 {
        shares::boundary(&self.col_shares, index)
    }

    /// Where the line after the row `index` stands, from 0 (the top of the node) to 1.
    pub fn row_boundary(&self, index: usize) -> f32 {
        shares::boundary(&self.row_shares, index)
    }

    /// Replaces the shares of the columns and of the rows.
    ///
    /// # Panics
    ///
    /// If there is not one share per column and one per row, or a share is negative or not
    /// finite.
    pub fn set_shares(&mut self, col_shares: Vec<f32>, row_shares: Vec<f32>) {
        assert_eq!(col_shares.len(), self.columns(), "one share per column");
        assert_eq!(row_shares.len(), self.rows(), "one share per row");
        assert!(
            col_shares
                .iter()
                .chain(&row_shares)
                .all(|s| s.is_finite() && *s >= 0.0),
            "shares are finite and not negative"
        );
        self.col_shares = col_shares;
        self.row_shares = row_shares;
    }

    /// Sets how the number of columns is chosen, and fits the shares to it.
    pub fn set_layout(&mut self, layout: GridLayout) {
        self.layout = layout;
        self.fit_shares();
    }

    /// Brings the shares to one per column and one per row.
    pub(crate) fn fit_shares(&mut self) {
        let (columns, rows) = (self.columns(), self.rows());
        shares::fit(&mut self.col_shares, columns);
        shares::fit(&mut self.row_shares, rows);
    }
}

/// The number of columns of a grid of `len` children.
fn columns(layout: GridLayout, len: usize) -> usize {
    match layout {
        GridLayout::Auto => (len as f64).sqrt().ceil() as usize,
        GridLayout::Columns(columns) => columns.min(len),
    }
    .max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid(len: usize) -> GridNode {
        GridNode::new(GridLayout::Auto, (0..len).map(NodeIndex).collect())
    }

    #[test]
    fn auto_columns_are_the_rounded_up_square_root() {
        let expected = [
            (1, 1, 1),
            (2, 2, 1),
            (3, 2, 2),
            (4, 2, 2),
            (5, 3, 2),
            (9, 3, 3),
        ];
        for (len, columns, rows) in expected {
            let grid = grid(len);
            assert_eq!((grid.columns(), grid.rows()), (columns, rows), "{len}");
            assert_eq!(grid.col_shares().len(), columns);
            assert_eq!(grid.row_shares().len(), rows);
        }
        assert_eq!(grid(10).columns(), 4);
    }

    #[test]
    fn fixed_columns() {
        let mut grid = grid(5);
        grid.set_layout(GridLayout::Columns(1));
        assert_eq!((grid.columns(), grid.rows()), (1, 5));
        grid.set_layout(GridLayout::Columns(9));
        assert_eq!((grid.columns(), grid.rows()), (5, 1));
        grid.set_layout(GridLayout::Columns(0));
        assert_eq!(grid.columns(), 1);
    }

    #[test]
    fn cells_fill_row_by_row() {
        let grid = grid(5);
        assert_eq!(grid.cell(0), (0, 0));
        assert_eq!(grid.cell(2), (2, 0));
        assert_eq!(grid.cell(3), (0, 1));
        assert_eq!(grid.cell(4), (1, 1));
    }

    #[test]
    fn a_new_column_gets_the_mean_share() {
        let mut grid = grid(4);
        grid.set_shares(vec![1.0, 3.0], vec![1.0, 1.0]);
        grid.children.push(NodeIndex(4));
        grid.fit_shares();
        assert_eq!(grid.col_shares(), &[1.0, 3.0, 2.0]);
        assert_eq!(grid.row_shares(), &[1.0, 1.0]);
        assert!((grid.col_boundary(0) - 1.0 / 6.0).abs() < 1e-6);
    }

    #[test]
    fn a_column_line_moves_its_pair_only() {
        let mut grid = grid(9);
        shares::move_boundary(&mut grid.col_shares, 0, 0.1, 0.05);
        assert!((grid.col_fraction(0) - (1.0 / 3.0 + 0.1)).abs() < 1e-6);
        assert!((grid.col_fraction(2) - 1.0 / 3.0).abs() < 1e-6);
        assert!((grid.row_fraction(0) - 1.0 / 3.0).abs() < 1e-6);
        shares::even_pair(&mut grid.col_shares, 0);
        assert!((grid.col_fraction(0) - grid.col_fraction(1)).abs() < 1e-6);
    }
}
