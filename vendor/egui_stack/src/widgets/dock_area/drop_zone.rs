use egui::{Pos2, Rect};

use crate::{AllowedSplits, Split};

/// The region of a leaf a dragged tab is dropped on with
/// [`OverlayType::Zones`](crate::OverlayType::Zones): a band along one of its edges, which splits
/// the leaf on that side, or the rest of it, which adds the tab to the leaf.
///
/// [`DropZone::at`] is the hit-test the dock uses, public so that an application drawing its own
/// drop targets (another dock, a window of its own) can pick zones the same way.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DropZone {
    /// The middle of the leaf: the tab joins its tabs.
    Center,
    /// The band along the left edge: the tab goes left of the leaf.
    Left,
    /// The band along the right edge: the tab goes right of the leaf.
    Right,
    /// The band along the top edge: the tab goes above the leaf.
    Top,
    /// The band along the bottom edge: the tab goes below the leaf.
    Bottom,
}

impl DropZone {
    /// The zone of `rect` under `pointer`.
    ///
    /// Each edge has a band covering `edge_coverage` of the leaf's width (left and right) or
    /// height (top and bottom), clamped to `0.0..=0.5`. In a band, the edge nearest the pointer,
    /// relative to the leaf's size, wins; anywhere else is [`DropZone::Center`]. Only the edges
    /// `allowed` splits permit have a band. A pointer outside `rect` counts as on its border, and
    /// an empty `rect` is all center.
    pub fn at(rect: Rect, pointer: Pos2, edge_coverage: f32, allowed: AllowedSplits) -> Self {
        let edge_coverage = edge_coverage.clamp(0.0, 0.5);
        if !(rect.width() > 0.0 && rect.height() > 0.0) || edge_coverage == 0.0 {
            return Self::Center;
        }
        let x = ((pointer.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
        let y = ((pointer.y - rect.top()) / rect.height()).clamp(0.0, 1.0);
        let left_right = matches!(allowed, AllowedSplits::All | AllowedSplits::LeftRightOnly);
        let top_bottom = matches!(allowed, AllowedSplits::All | AllowedSplits::TopBottomOnly);

        let mut best = (Self::Center, edge_coverage);
        for (zone, distance, enabled) in [
            (Self::Left, x, left_right),
            (Self::Right, 1.0 - x, left_right),
            (Self::Top, y, top_bottom),
            (Self::Bottom, 1.0 - y, top_bottom),
        ] {
            if enabled && distance < best.1 {
                best = (zone, distance);
            }
        }
        best.0
    }

    /// The split this zone makes, `None` for [`DropZone::Center`].
    pub const fn split(self) -> Option<Split> {
        match self {
            Self::Center => None,
            Self::Left => Some(Split::Left),
            Self::Right => Some(Split::Right),
            Self::Top => Some(Split::Above),
            Self::Bottom => Some(Split::Below),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::DropZone;
    use crate::{AllowedSplits, Split};
    use egui::{Rect, pos2};

    const LEAF: Rect = Rect::from_min_max(pos2(100.0, 50.0), pos2(500.0, 250.0));

    fn at(x: f32, y: f32) -> DropZone {
        DropZone::at(LEAF, pos2(x, y), 0.25, AllowedSplits::All)
    }

    #[test]
    fn the_middle_is_the_center() {
        assert_eq!(at(300.0, 150.0), DropZone::Center);
        // Just inside each band's inner limit: a quarter of the width, of the height.
        assert_eq!(at(201.0, 150.0), DropZone::Center);
        assert_eq!(at(399.0, 150.0), DropZone::Center);
        assert_eq!(at(300.0, 101.0), DropZone::Center);
        assert_eq!(at(300.0, 199.0), DropZone::Center);
    }

    #[test]
    fn each_edge_has_its_band() {
        assert_eq!(at(110.0, 150.0), DropZone::Left);
        assert_eq!(at(199.0, 150.0), DropZone::Left);
        assert_eq!(at(490.0, 150.0), DropZone::Right);
        assert_eq!(at(401.0, 150.0), DropZone::Right);
        assert_eq!(at(300.0, 55.0), DropZone::Top);
        assert_eq!(at(300.0, 99.0), DropZone::Top);
        assert_eq!(at(300.0, 245.0), DropZone::Bottom);
        assert_eq!(at(300.0, 201.0), DropZone::Bottom);
    }

    #[test]
    fn in_a_corner_the_nearer_edge_wins() {
        // 5% of the width from the left, 10% of the height from the top.
        assert_eq!(at(120.0, 70.0), DropZone::Left);
        // 10% of the width from the left, 5% of the height from the top.
        assert_eq!(at(140.0, 60.0), DropZone::Top);
        assert_eq!(at(495.0, 240.0), DropZone::Right);
        assert_eq!(at(460.0, 245.0), DropZone::Bottom);
    }

    #[test]
    fn a_pointer_outside_counts_as_on_the_border() {
        assert_eq!(at(20.0, 150.0), DropZone::Left);
        assert_eq!(at(300.0, 900.0), DropZone::Bottom);
    }

    #[test]
    fn only_allowed_splits_have_bands() {
        let near_left_top = pos2(110.0, 55.0);
        let pick = |allowed| DropZone::at(LEAF, near_left_top, 0.25, allowed);
        assert_eq!(pick(AllowedSplits::LeftRightOnly), DropZone::Left);
        assert_eq!(pick(AllowedSplits::TopBottomOnly), DropZone::Top);
        assert_eq!(pick(AllowedSplits::None), DropZone::Center);
    }

    #[test]
    fn the_coverage_sets_the_band_width() {
        let pointer = pos2(180.0, 150.0); // 20% of the width from the left.
        assert_eq!(
            DropZone::at(LEAF, pointer, 0.1, AllowedSplits::All),
            DropZone::Center
        );
        assert_eq!(
            DropZone::at(LEAF, pointer, 0.3, AllowedSplits::All),
            DropZone::Left
        );
        // Clamped to a half: the whole leaf is bands, but for its exact middle.
        assert_eq!(
            DropZone::at(LEAF, pos2(290.0, 150.0), 2.0, AllowedSplits::All),
            DropZone::Left
        );
        assert_eq!(
            DropZone::at(LEAF, pointer, 0.0, AllowedSplits::All),
            DropZone::Center
        );
    }

    #[test]
    fn an_empty_leaf_is_all_center() {
        let empty = Rect::from_min_size(pos2(10.0, 10.0), egui::Vec2::ZERO);
        assert_eq!(
            DropZone::at(empty, pos2(10.0, 10.0), 0.25, AllowedSplits::All),
            DropZone::Center
        );
    }

    #[test]
    fn zones_map_to_splits() {
        assert_eq!(DropZone::Center.split(), None);
        assert_eq!(DropZone::Left.split(), Some(Split::Left));
        assert_eq!(DropZone::Right.split(), Some(Split::Right));
        assert_eq!(DropZone::Top.split(), Some(Split::Above));
        assert_eq!(DropZone::Bottom.split(), Some(Split::Below));
    }
}
