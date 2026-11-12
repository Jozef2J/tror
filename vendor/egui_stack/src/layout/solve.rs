//! The layout's solver: the rect of every zone, one split at a time.

use std::collections::HashMap;

use egui::{Pos2, Rect, pos2};

use super::{Content, Dir, Drops, Fold, Layout, Zone, ZoneKind, ZoneName};
use crate::sections::plan::give_in_turn;

/// What the solver needs besides the layout.
#[derive(Clone, Debug, PartialEq)]
pub struct Solve {
    /// A folded zone's strip breadth, along its parent's axis.
    pub strip: f32,
    /// The size each zone's content asked for last frame, along its
    /// parent's axis: a least size for zones with
    /// [`Constraints::fit_content`](super::Constraints::fit_content).
    pub content: HashMap<ZoneName, f32>,
}

impl Default for Solve {
    fn default() -> Self {
        Self {
            strip: 24.0,
            content: HashMap::new(),
        }
    }
}

/// How a zone is shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Shown {
    /// In the flow, at its solved size.
    Open,
    /// Folded by the user ([`ZoneState::folded`](super::ZoneState::folded)):
    /// its strip only.
    Folded,
    /// Folded by the solver, short of room
    /// ([`Constraints::auto_fold`](super::Constraints::auto_fold)): its strip
    /// only, until the room comes back.
    AutoFolded,
    /// Its strip in the flow, its body over the neighbour towards the zone
    /// that grows ([`ZoneState::over`](super::ZoneState::over)).
    Over,
    /// Not shown, taking no room.
    Hidden,
}

/// One zone as solved.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct LaidZone {
    /// Its name.
    pub name: ZoneName,
    /// Where its content goes: its strip when folded, its body when over,
    /// an empty rect when hidden. Cut at the parent's edge when even the
    /// least sizes do not fit.
    pub rect: Rect,
    /// How it is shown.
    pub shown: Shown,
    /// Its strip, when folded or over.
    pub strip: Option<Rect>,
    /// Its least size along its parent's axis, as solved (0 at the root).
    pub least: f32,
    /// Its largest size along its parent's axis, as solved (infinite when
    /// unbounded, and at the root).
    pub most: f32,
    /// Its content, for a leaf; `None` for a split.
    pub content: Option<Content>,
    /// Whether a dragged item may be dropped in it.
    pub drops: Drops,
    /// Whether a tab dropped on its strip, while folded, joins its dock
    /// ([`Constraints::strip_drops`](super::Constraints::strip_drops)).
    pub strip_drops: bool,
}

impl LaidZone {
    /// The dock it shows, for a leaf holding [`Content::Dock`].
    pub fn dock(&self) -> Option<&str> {
        match &self.content {
            Some(Content::Dock(name)) => Some(name),
            _ => None,
        }
    }
}

/// What changed in a layout while a [`LayoutArea`](super::LayoutArea) drew
/// it: the user's acts, each already applied to the [`Layout`].
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum LayoutEvent {
    /// A sash, or the keyboard, gave the zone this size in points.
    Resized {
        /// The zone.
        zone: ZoneName,
        /// Its new [`ZoneState::size`](super::ZoneState::size).
        size: f32,
    },
    /// The zone went back to its default size (a sash's double click or
    /// Enter).
    Reset {
        /// The zone.
        zone: ZoneName,
    },
    /// The zone folded to its strip: a sash dragged it under half its
    /// least size, its grip was clicked while it lay over, or it lay over
    /// and Escape or a click outside closed it.
    Folded {
        /// The zone.
        zone: ZoneName,
    },
    /// The zone unfolded, back in the flow: its strip's grip was clicked.
    Unfolded {
        /// The zone.
        zone: ZoneName,
    },
    /// The flyout asked to close (Escape, a click outside it, its strip's
    /// grip, its zone unfolded): the application stops giving it to
    /// [`LayoutArea::flyout`](super::LayoutArea::flyout).
    FlyoutClosed {
        /// The flyout's key.
        key: String,
    },
    /// The flyout's free edge gave it this breadth, kept in
    /// [`Layout::flyouts`].
    FlyoutResized {
        /// The flyout's key.
        key: String,
        /// Its breadth, in points.
        size: f32,
    },
}

/// Where every zone goes: the solver's output, parents first. Zones under a
/// folded or hidden zone are not laid.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct Laid {
    /// The zones, parents first.
    pub zones: Vec<LaidZone>,
    /// What the user changed this frame, when drawn by a
    /// [`LayoutArea`](super::LayoutArea); empty from [`Layout::solve`].
    pub events: Vec<LayoutEvent>,
    /// The flyout's rect, when one was drawn.
    pub flyout: Option<Rect>,
}

impl Laid {
    /// The zone named `name`, when laid.
    pub fn get(&self, name: &str) -> Option<&LaidZone> {
        self.zones.iter().find(|zone| zone.name == *name)
    }

    /// The rect of the zone named `name`, when laid.
    pub fn rect(&self, name: &str) -> Option<Rect> {
        self.get(name).map(|zone| zone.rect)
    }
}

impl Layout {
    /// Lays the zones out in `rect`; the root takes all of it. Pure: the
    /// layout is not changed, and a zone the solver shows smaller, or folds,
    /// keeps its state.
    pub fn solve(&self, rect: Rect, solve: &Solve) -> Laid {
        let mut laid = Laid::default();
        laid.zones.push(LaidZone {
            name: self.root.name.clone(),
            rect,
            shown: Shown::Open,
            strip: None,
            least: 0.0,
            most: f32::INFINITY,
            content: leaf_content(&self.root),
            drops: self.root.constraints.drops,
            strip_drops: false,
        });
        lay_children(&self.root, rect, solve, &mut laid);
        laid
    }
}

/// A child's place in a split, before positions.
#[derive(Clone, Copy, Debug)]
struct Sized {
    shown: Shown,
    least: f32,
    most: f32,
    /// Its size in the flow.
    size: f32,
    /// Its body's size, when over.
    body: f32,
}

fn lay_children(zone: &Zone, rect: Rect, solve: &Solve, laid: &mut Laid) {
    let ZoneKind::Split { dir, children } = &zone.kind else {
        return;
    };
    if children.is_empty() {
        return;
    }
    let length = along(*dir, rect.size());
    let (sized, taker) = sizes(children, length, solve);
    let mut pos = along(*dir, rect.min.to_vec2());
    let start = pos;
    let end = start + length;
    for (i, (child, s)) in children.iter().zip(&sized).enumerate() {
        let span = |a: f32, b: f32| span(*dir, rect, a.clamp(start, end), b.clamp(start, end));
        let flow = span(pos, pos + s.size);
        let (child_rect, strip) = match s.shown {
            Shown::Open => (flow, None),
            Shown::Folded | Shown::AutoFolded => (flow, Some(flow)),
            Shown::Hidden => (span(pos, pos), None),
            Shown::Over => {
                let towards_end = taker.is_none_or(|t| i < t);
                let body = if towards_end {
                    span(pos + s.size, pos + s.size + s.body)
                } else {
                    span(pos - s.body, pos)
                };
                (body, Some(flow))
            }
        };
        pos += s.size;
        laid.zones.push(LaidZone {
            name: child.name.clone(),
            rect: child_rect,
            shown: s.shown,
            strip,
            least: s.least,
            most: s.most,
            content: leaf_content(child),
            drops: child.constraints.drops,
            strip_drops: child.constraints.strip_drops,
        });
        if matches!(s.shown, Shown::Open | Shown::Over) {
            lay_children(child, child_rect, solve, laid);
        }
    }
}

/// Each child's size along a split of `length`, and the child that takes
/// the rest (the one that grows, else the last open one): lying-over zones
/// open towards it.
fn sizes(children: &[Zone], length: f32, solve: &Solve) -> (Vec<Sized>, Option<usize>) {
    let grow = children
        .iter()
        .position(|c| c.constraints.grow && !c.state.hidden);
    let mut auto_folded: Vec<usize> = Vec::new();
    loop {
        let (sized, short) = fit(children, length, solve, grow, &auto_folded);
        if short <= 0.0 {
            let taker = taker(&sized, grow);
            return (sized, taker);
        }
        // Even the least sizes do not fit: the next zone that opted in
        // folds, lowest priority first, then the nearest to the one that
        // takes the rest.
        let taker = taker(&sized, grow);
        let next = giving_order(children, &sized, taker)
            .into_iter()
            .find(|&i| {
                let c = &children[i].constraints;
                c.fold == Fold::ToStrip && c.auto_fold && !auto_folded.contains(&i)
            });
        match next {
            Some(i) => auto_folded.push(i),
            None => return (sized, taker),
        }
    }
}

/// The child that takes the rest: the one that grows, else the last open one.
fn taker(sized: &[Sized], grow: Option<usize>) -> Option<usize> {
    grow.or_else(|| sized.iter().rposition(|s| s.shown == Shown::Open))
}

/// The open children other than `taker`, in the order they give way: the
/// lowest priority first, then the nearest to `taker`, then the first.
fn giving_order(children: &[Zone], sized: &[Sized], taker: Option<usize>) -> Vec<usize> {
    let Some(taker) = taker else {
        return Vec::new();
    };
    let mut order: Vec<usize> = (0..children.len())
        .filter(|&i| i != taker && sized[i].shown == Shown::Open)
        .collect();
    order.sort_by_key(|&i| (children[i].constraints.priority, i.abs_diff(taker), i));
    order
}

/// The sizes with the children in `auto_folded` folded, and how much the
/// taker's least size is still short of.
fn fit(
    children: &[Zone],
    length: f32,
    solve: &Solve,
    grow: Option<usize>,
    auto_folded: &[usize],
) -> (Vec<Sized>, f32) {
    let fixed = |i: usize, c: &Zone| {
        Some(i) != grow && (c.state.hidden || c.state.folded || auto_folded.contains(&i))
    };
    let flexible = children
        .iter()
        .enumerate()
        .filter(|&(i, c)| !fixed(i, c) && (!c.state.over || Some(i) == grow))
        .count()
        .max(1);
    let even = length / flexible as f32;
    let mut sized: Vec<Sized> = children
        .iter()
        .enumerate()
        .map(|(i, child)| {
            let c = &child.constraints;
            let mut least = c.min.map_or(0.0, |s| s.points(length)).max(0.0);
            if c.fit_content
                && let Some(&asked) = solve.content.get(&child.name)
            {
                least = least.max(asked);
            }
            let most = c.max.map_or(f32::INFINITY, |s| s.points(length)).max(least);
            let wanted = child
                .state
                .size
                .or_else(|| c.default.map(|s| s.points(length)))
                .unwrap_or(even);
            let open = wanted.clamp(least, most);
            let is_grow = Some(i) == grow;
            let (shown, size) = if child.state.hidden && !is_grow {
                (Shown::Hidden, 0.0)
            } else if auto_folded.contains(&i) {
                (Shown::AutoFolded, solve.strip)
            } else if child.state.folded && !is_grow {
                (Shown::Folded, solve.strip)
            } else if child.state.over && !is_grow {
                (Shown::Over, solve.strip)
            } else {
                (Shown::Open, open)
            };
            Sized {
                shown,
                least,
                most,
                size,
                body: open,
            }
        })
        .collect();
    let Some(taker) = taker(&sized, grow) else {
        return (sized, 0.0);
    };
    let others: f32 = (0..sized.len())
        .filter(|&i| i != taker)
        .map(|i| sized[i].size)
        .sum();
    let rest = length - others;
    let order = giving_order(children, &sized, taker.into());
    let mut short = 0.0;
    if rest < sized[taker].least {
        // The others give way, in turn, each down to its least size.
        let mut sizes: Vec<f32> = sized.iter().map(|s| s.size).collect();
        let owed = sized[taker].least - rest;
        let given = give_in_turn(&mut sizes, |i| sized[i].least, order, owed);
        for (s, size) in sized.iter_mut().zip(sizes) {
            s.size = size;
        }
        short = owed - given;
        sized[taker].size = sized[taker].least;
    } else if rest > sized[taker].most {
        // The others take the surplus up to their largest size, the
        // highest priority first; what is left stays with the zone that
        // grows, or empty after the last zone.
        let mut surplus = rest - sized[taker].most;
        let mut by_priority = order;
        by_priority.sort_by_key(|&i| std::cmp::Reverse(children[i].constraints.priority));
        for i in by_priority {
            let took = (sized[i].most - sized[i].size).max(0.0).min(surplus);
            sized[i].size += took;
            surplus -= took;
        }
        sized[taker].size = sized[taker].most;
        if grow.is_some() {
            sized[taker].size += surplus;
        }
    } else {
        sized[taker].size = rest;
    }
    (sized, short)
}

fn leaf_content(zone: &Zone) -> Option<Content> {
    match &zone.kind {
        ZoneKind::Leaf { content } => Some(content.clone()),
        ZoneKind::Split { .. } => None,
    }
}

pub(super) fn along(dir: Dir, v: egui::Vec2) -> f32 {
    match dir {
        Dir::Horizontal => v.x,
        Dir::Vertical => v.y,
    }
}

/// The part of `rect` between `a` and `b` along `dir`.
pub(super) fn span(dir: Dir, rect: Rect, a: f32, b: f32) -> Rect {
    let (min, max): (Pos2, Pos2) = match dir {
        Dir::Horizontal => (pos2(a, rect.min.y), pos2(b, rect.max.y)),
        Dir::Vertical => (pos2(rect.min.x, a), pos2(rect.max.x, b)),
    };
    Rect::from_min_max(min, max)
}
