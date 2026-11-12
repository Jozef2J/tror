//! Collapsible sections stacked in a panel: each a title band (a disclosure mark, a title, a
//! count) over its own scrolling body, as a code editor's side bar stacks its views.
//!
//! - A click on a band, or Space or Enter while it has the keyboard, folds or opens its
//!   section; Left folds it and Right opens it. A folded section keeps its band in place; open
//!   sections fill the panel's height, so the bands of folded sections below the last open one
//!   sit at the panel's bottom.
//! - A section's height is set by hand, in points, or sized by the list: those never set by
//!   hand share what the others leave, and the last open section always takes what is left. Each
//!   keeps a least height (two rows by default, [`Section::min_height`]) and scrolls its own
//!   body. One may have a greatest height too ([`Section::max_height`], [`Section::max_share`]):
//!   sized by the list or by hand, it never grows past it, the others taking the room it
//!   leaves.
//! - The line above a band, between two open sections, is a boundary (the first open section
//!   above is the one it resizes, past any folded ones). It follows the pointer when dragged, or
//!   moves by a row with Up and Down while it has the keyboard. Sections give way in turn, the
//!   nearest first, never in proportion, and it stops once they are all at their least
//!   ([`plan`]). A double click, or Enter, drops the heights set by hand and any empty room.
//! - In a panel too short, sections sized by the list give way first, then those set by hand,
//!   from the bottom up; their stored heights come back with the room, until the next drag
//!   aligns them on what is shown. When even the least heights do not fit, the whole stack
//!   scrolls, with a scroll bar then only.
//! - [`Sections::grab_titles`] (off by default) makes the titles drag too: a press that moves
//!   resizes as the line above does, one that does not folds, and a double click drops the
//!   heights set by hand.
//! - The pull's modifiers ([`Sections::pull_modifiers`], Ctrl with a drag by default, Cmd on
//!   macOS) move the sections below as a block, heights kept: only the section above changes,
//!   and empty room opens under the last. The empty room is kept, with the layout, until a drag
//!   takes it back or a double click drops it.
//! - [`Sections::keep_titles`] (off by default) leaves a title where it is when its section
//!   folds or opens ([`plan::keep_title`]).
//! - [`Sections::fill`] (on by default) off sizes the stack by its content: an open section
//!   sized by the list is never taller than what its body lays out, the folded bands follow
//!   right under it, and the room left stays empty under the last band ([`plan::fit_content`]).
//!   A drag still moves a band: the section above grows past its content, into that room, the
//!   last open one included, and keeps the height it is given until a double click.
//! - [`Sections::bar_style`] and [`Sections::bar_track_inset`] give the scroll bars of the
//!   bodies and of the stack a style of their own (their colours, say) and an inset along
//!   their track, the content keeping the `Ui`'s style.
//! - A body is scrolled by the stack ([`Stack::section`]), windowed over rows of one height, so
//!   that only those in view are drawn ([`Stack::rows`]), or left to an application's own view
//!   ([`Stack::bare`]). A wheel pushing past the end of a body's scroll stops there: neither the
//!   stack nor a parent moves.
//! - What is folded, the heights set by hand and the empty room are remembered in egui's memory
//!   under the stack's name (persisted with the `serde` feature), and readable as a [`Layout`]
//!   an application keeps with its own.
//!
//! The bands and boundaries are painted by a [`DockPainter`]
//! ([`DockPainter::section_band`], [`DockPainter::section_boundary`]), [`StylePainter`] by
//! default, from the `Ui`'s [`egui::Style`]: the painter that gives a dock its look gives its
//! section stacks the same.
//!
//! **Named and observable.** A band is a button named by its title and count ("Live, 3"),
//! expanded or not for assistive technology; a boundary is a splitter named after the section
//! above it ([`Labels::resize_section`], "Resize Live"), its value the height of that section's
//! body. [`laid_out`] gives each section as drawn on the last pass, with its height and whether
//! that was set by hand, so a test or an agent reads the stack rather than pixels.
//!
//! ```
//! use egui_stack::sections::{Section, Sections, laid_out};
//!
//! # fn main() {
//! # if cfg!(miri) { return; } // A whole UI: too slow under Miri.
//! let ctx = egui::Context::default();
//! let mut out = ctx.run_ui(Default::default(), |ui| {
//!     Sections::new("sessions").show(ui, |stack| {
//!         stack.section(Section::new("live", "Live").count(2), |ui| {
//!             ui.label("build");
//!             ui.label("tests");
//!         });
//!         stack.section(Section::new("idle", "Idle").count(0).open(false), |ui| {
//!             ui.weak("No idle session");
//!         });
//!     });
//! });
//! out.textures_delta.clear();
//! let live = &laid_out(&ctx, "sessions")[0];
//! assert!(live.open && live.body.is_some());
//! # }
//! ```

use std::collections::BTreeMap;
use std::ops::Range;
use std::sync::Arc;

use egui::accesskit::{Action, Role};
use egui::scroll_area::{DragScroll, ScrollBarVisibility, ScrollSource};
use egui::style::ScrollStyle;
use egui::{
    Context, CursorIcon, EventFilter, Id, Key, Modifiers, PointerButton, Rect, Response, Sense,
    TextStyle, Ui, UiBuilder, WidgetInfo, pos2, vec2,
};

use crate::{DockPainter, Labels, PartState, SectionBandPaint, SectionBoundaryPaint, StylePainter};

pub mod plan;

use plan::{Asked, DragMode, Placed};

/// What a stack remembers of one section.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub struct Remembered {
    /// Whether the section is open.
    pub open: bool,
    /// Its height set by hand, in points; none when sized by the list. Kept while folded, so a
    /// section opens again at its size, and while it is the last open one, which takes what the
    /// others leave.
    #[cfg_attr(feature = "serde", serde(default))]
    pub height: Option<f32>,
}

/// What a stack remembers, by section key: kept in egui's memory under the stack's name, and
/// readable and writable whole ([`Layout::load`], [`Layout::store`]) for an application that
/// keeps it with its own layout.
#[derive(Clone, Debug, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub struct Layout {
    /// Each section's fold and height, by key.
    pub sections: BTreeMap<String, Remembered>,
    /// The empty room kept under the last section, in points.
    #[cfg_attr(feature = "serde", serde(default))]
    pub empty: f32,
}

impl Layout {
    /// The layout a stack named `name` remembers, if any.
    pub fn load(ctx: &Context, name: &str) -> Option<Layout> {
        let id = memory_id(name);
        #[cfg(feature = "serde")]
        return ctx.data_mut(|d| d.get_persisted::<Layout>(id));
        #[cfg(not(feature = "serde"))]
        return ctx.data(|d| d.get_temp::<Layout>(id));
    }

    /// Makes it the layout of the stack named `name` (shown so from the next pass).
    pub fn store(&self, ctx: &Context, name: &str) {
        let id = memory_id(name);
        let layout = self.clone();
        #[cfg(feature = "serde")]
        ctx.data_mut(|d| d.insert_persisted(id, layout));
        #[cfg(not(feature = "serde"))]
        ctx.data_mut(|d| d.insert_temp(id, layout));
    }

    /// Whether a section is open, if the layout knows it.
    pub fn is_open(&self, key: &str) -> Option<bool> {
        self.sections.get(key).map(|s| s.open)
    }

    /// Opens or folds a section.
    pub fn set_open(&mut self, key: &str, open: bool) {
        self.sections
            .entry(key.to_owned())
            .or_insert(Remembered { open, height: None })
            .open = open;
    }

    /// A section's height set by hand, if it has one.
    pub fn height(&self, key: &str) -> Option<f32> {
        self.sections.get(key).and_then(|s| s.height)
    }

    /// Sets a section's height by hand, as a drag would, or gives it back to the list. It opens
    /// as the stack first shows it when the layout does not know it yet.
    pub fn set_height(&mut self, key: &str, height: Option<f32>) {
        match self.sections.get_mut(key) {
            Some(s) => s.height = height,
            None => {
                self.sections
                    .insert(key.to_owned(), Remembered { open: true, height });
            }
        }
    }

    /// Drops every height set by hand and the empty room (a boundary's double click): the
    /// sections share the panel again.
    pub fn reset_heights(&mut self) {
        for s in self.sections.values_mut() {
            s.height = None;
        }
        self.empty = 0.0;
    }
}

fn memory_id(name: &str) -> Id {
    Id::unique(("egui_stack.sections", name))
}

/// A section as laid out on the last pass ([`laid_out`]).
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct Laid {
    /// Its key.
    pub key: String,
    /// Its title, in its own case.
    pub title: String,
    /// The count on its band, if any.
    pub count: Option<u64>,
    /// Whether it is open.
    pub open: bool,
    /// Its body's height, in points (0 when folded).
    pub height: f32,
    /// Whether that height is its own, set by hand. Never for the last open section, which takes
    /// what the others leave.
    pub by_hand: bool,
    /// Its band.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub band: Rect,
    /// Its body, when open.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub body: Option<Rect>,
    /// How far its body is scrolled, in points (0 when folded, and for a bare body, whose scroll
    /// is the application's).
    pub scroll: f32,
    /// The rows drawn on that pass, when windowed and open.
    pub window: Option<Range<usize>>,
    /// The empty room under it, in points: only the last section has any (all of it, when no
    /// section is open).
    pub empty: f32,
    /// How a drag under way resizes it, if one does.
    pub drag: Option<DragMode>,
}

/// The heights kept by hand when a drag or a key press began: what its plans are pinned against,
/// so that a section the drag leaves alone stays in the list.
#[derive(Clone, Debug)]
struct Hands(Vec<Option<f32>>);

impl Hands {
    /// What the stack asks for, with the heights kept by hand as they were.
    fn asked(&self, stack: &Shown<'_>) -> Vec<Asked> {
        stack
            .asked
            .iter()
            .zip(&self.0)
            .map(|(a, &hand)| Asked { hand, ..*a })
            .collect()
    }
}

/// A drag on a boundary: where the pointer was pressed and where the boundary was then, so that
/// it stays under the pointer without drifting. Each pass plans from the stack as shown: what
/// the drag shrank stays shrunk, and the room a move back gives goes where a fresh drag would
/// put it.
#[derive(Clone, Debug)]
struct Drag {
    origin: f32,
    /// The heights above the boundary, summed, when the drag began: how far it has moved since
    /// is what they are now, less this.
    top: f32,
    hands: Hands,
    /// Where the pointer was on the last pass: no move goes further than the pointer has since.
    last: f32,
}

/// A click on a title, for as long as a second one would be a double click.
#[derive(Clone, Debug)]
struct TitleClick {
    key: String,
    /// The band it landed on, where the second click will land.
    band: Rect,
    /// When, in egui's time.
    at: f64,
}

/// What a stack keeps between passes, besides its [`Layout`], under an id made from its name.
fn temp_id(name: &str, what: &str) -> Id {
    Id::unique(("egui_stack.sections", name, what))
}

fn get_temp<T: 'static + Clone + Send + Sync>(ctx: &Context, name: &str, what: &str) -> Option<T> {
    ctx.data(|d| d.get_temp::<T>(temp_id(name, what)))
}

fn set_temp<T: 'static + Clone + Send + Sync>(ctx: &Context, name: &str, what: &str, value: T) {
    ctx.data_mut(|d| d.insert_temp(temp_id(name, what), value));
}

fn remove_temp<T: 'static + Clone + Send + Sync>(ctx: &Context, name: &str, what: &str) {
    ctx.data_mut(|d| d.remove::<T>(temp_id(name, what)));
}

const LAID: &str = "laid";
const DRAG: &str = "drag";
const TITLE_CLICK: &str = "title_click";
const CONTENT: &str = "content";

/// The sections of the stack named `name` as drawn on the last pass, in order (empty when it
/// was not drawn).
pub fn laid_out(ctx: &Context, name: &str) -> Vec<Laid> {
    get_temp(ctx, name, LAID).unwrap_or_default()
}

/// The height of a row when the application names none ([`Section::row_height`]): the style's
/// interactive height, or the body text's when that is taller.
pub fn row_height(ui: &Ui) -> f32 {
    ui.spacing()
        .interact_size
        .y
        .max(ui.text_style_height(&TextStyle::Body))
}

/// One section: a key (stable, what the layout remembers it by), a title, an optional count, its
/// least height, its rows' height and whether it starts open.
#[derive(Clone, Debug)]
pub struct Section {
    key: String,
    title: String,
    count: Option<u64>,
    min_height: Option<f32>,
    max_height: Option<Most>,
    row_height: Option<f32>,
    open: bool,
}

/// The most a section's body takes ([`Section::max_height`], [`Section::max_share`]).
#[derive(Clone, Copy, Debug)]
enum Most {
    Points(f32),
    Share(f32),
}

impl Most {
    /// In points, for a stack `stack` points tall (infinite: no bound).
    fn points(self, stack: f32) -> f32 {
        match self {
            Most::Points(points) => points,
            Most::Share(share) if stack.is_finite() => share * stack,
            Most::Share(_) => f32::INFINITY,
        }
    }
}

impl Section {
    /// An open section with no count.
    pub fn new(key: impl Into<String>, title: impl Into<String>) -> Self {
        Section {
            key: key.into(),
            title: title.into(),
            count: None,
            min_height: None,
            max_height: None,
            row_height: None,
            open: true,
        }
    }

    /// The count on its band (`99+` past 99 with the default painter; its name says the real
    /// count).
    pub fn count(mut self, count: u64) -> Self {
        self.count = Some(count);
        self
    }

    /// Its body's least height, in points. By default, room for two rows; for a windowed
    /// section ([`Stack::rows`]), two rows, or all of them when it has fewer.
    pub fn min_height(mut self, points: f32) -> Self {
        self.min_height = Some(points.max(0.0));
        self
    }

    /// Its body's greatest height, in points (no bound by default). Sized by the list, it never
    /// grows past it, and the room it leaves goes to the other sections, or stays empty under the
    /// last band when none can take it; a drag on its boundary stops there. Never below its least
    /// height ([`Section::min_height`]): the least wins.
    pub fn max_height(mut self, points: f32) -> Self {
        self.max_height = (!points.is_nan()).then_some(Most::Points(points.max(0.0)));
        self
    }

    /// [`Section::max_height`] as a share of the stack's height (its bands included): `0.5`, at
    /// most half of it. Never below its least height either.
    pub fn max_share(mut self, share: f32) -> Self {
        self.max_height = share.is_finite().then_some(Most::Share(share.max(0.0)));
        self
    }

    /// The height of its rows, in points, at least one (by default the style's, [`row_height`]):
    /// what its least height counts, and what [`Stack::rows`] windows by.
    pub fn row_height(mut self, points: f32) -> Self {
        self.row_height = Some(points.max(1.0));
        self
    }

    /// Whether it is open until the user says otherwise (open by default); the layout
    /// remembered wins.
    pub fn open(mut self, open: bool) -> Self {
        self.open = open;
        self
    }

    /// Its band's accessible name: the title, and the count when there is one ("Live, 3").
    fn name(&self) -> String {
        match self.count {
            Some(n) => format!("{}, {n}", self.title),
            None => self.title.clone(),
        }
    }
}

/// Draws the rows in a window, handed their range.
type DrawRows<'a> = Box<dyn FnOnce(&mut Ui, Range<usize>) + 'a>;

/// What a section draws under its band.
enum Body<'a> {
    /// Anything, in a scroll area.
    Scrolled(Box<dyn FnOnce(&mut Ui) + 'a>),
    /// `count` rows of one height, in a scroll area, drawn by the window.
    Rows { count: usize, draw: DrawRows<'a> },
    /// An application's own view, which scrolls itself.
    Bare(Box<dyn FnOnce(&mut Ui) + 'a>),
}

/// What drawing a body says.
struct Drawn {
    /// How far it is scrolled.
    scroll: f32,
    /// How tall its content is, margins included: the height it asks for in a stack sized by
    /// its content ([`Sections::fill`]).
    content: f32,
    /// The rows drawn.
    window: Option<Range<usize>>,
    /// Whether the wheel over it stops there: it has a scroll of its own, with somewhere to go.
    holds_wheel: bool,
}

impl Body<'_> {
    /// Its least height, unless the section names one.
    fn least(&self, ui: &Ui, row: f32) -> f32 {
        match self {
            Body::Rows { count, .. } => {
                plan::rows_least(row, *count) + ui.spacing().scroll.content_margin.sum().y
            }
            Body::Scrolled(_) | Body::Bare(_) => 2.0 * row,
        }
    }

    /// Draws it in `ui`, as tall as the section. `salt` tells its scroll from the others';
    /// `row` is its rows' height; `bars` how its scroll bar is drawn.
    fn draw(self, ui: &mut Ui, salt: &str, row: f32, bars: &Bars) -> Drawn {
        let rect = ui.max_rect();
        let margin = ui.spacing().scroll.content_margin.sum().y;
        // The content's style: the bars may be drawn in another.
        let content = ui.style().clone();
        let area = bars.on(
            egui::ScrollArea::vertical()
                .id_salt(("egui_stack.sections.scroll", salt))
                .auto_shrink([false, false])
                .max_height(rect.height())
                .min_scrolled_height(0.0),
            rect,
        );
        match self {
            Body::Scrolled(draw) => {
                let out = bars.show(ui, |ui| {
                    area.show(ui, |ui| {
                        ui.set_style(content);
                        draw(ui);
                    })
                });
                Drawn {
                    scroll: out.state.clamped_offset().y,
                    content: out.content_size.y + margin,
                    window: None,
                    holds_wheel: scrolled(ui.ctx(), &out),
                }
            }
            Body::Rows { count, draw } => {
                // Rows touch, as a list's: their height is all that places them.
                ui.spacing_mut().item_spacing.y = 0.0;
                let content = ui.style().clone();
                let mut window = None;
                let out = bars.show(ui, |ui| {
                    area.show_rows(ui, row, count, |ui, range| {
                        ui.set_style(content);
                        window = Some(range.clone());
                        draw(ui, range);
                    })
                });
                Drawn {
                    scroll: out.state.clamped_offset().y,
                    content: count as f32 * row + margin,
                    window,
                    holds_wheel: scrolled(ui.ctx(), &out),
                }
            }
            Body::Bare(draw) => {
                draw(ui);
                Drawn {
                    scroll: 0.0,
                    content: ui.min_rect().height(),
                    window: None,
                    holds_wheel: true,
                }
            }
        }
    }
}

/// How the stack's scroll bars are drawn ([`Sections::bar_style`],
/// [`Sections::bar_track_inset`]).
#[derive(Clone, Default)]
struct Bars {
    style: Option<Arc<egui::Style>>,
    inset: f32,
}

impl Bars {
    /// The area, its bars inset along their track in an area shown in `rect`.
    fn on(&self, area: egui::ScrollArea, rect: Rect) -> egui::ScrollArea {
        if self.inset > 0.0 {
            let inset = vec2(
                self.inset.min(rect.width() / 2.0),
                self.inset.min(rect.height() / 2.0),
            );
            area.scroll_bar_rect(rect.shrink2(inset))
        } else {
            area
        }
    }

    /// Shows a scroll area in the bars' style, which egui paints its bars with; `show` sets
    /// the content's style back inside the area. The `Ui`'s spacing is kept: the bars' style
    /// changes how they look, not how anything is laid out.
    fn show<R>(&self, ui: &mut Ui, show: impl FnOnce(&mut Ui) -> R) -> R {
        let Some(style) = &self.style else {
            return show(ui);
        };
        let mut scoped = style.as_ref().clone();
        scoped.spacing = ui.spacing().clone();
        ui.scope(|ui| {
            ui.set_style(scoped);
            show(ui)
        })
        .inner
    }
}

/// After a body's scroll area: says whether it has somewhere to scroll, and asks for another
/// pass when its offset changed.
///
/// A scroll area clamps its offset after drawing and asks for no repaint, so a body grown, or
/// left with fewer rows, would show blank rows under its last one until something else
/// repaints.
fn scrolled<R>(ctx: &Context, out: &egui::scroll_area::ScrollAreaOutput<R>) -> bool {
    let id = out.id.with("egui_stack.sections.offset");
    let offset = out.state.clamped_offset().y;
    let before = ctx.data(|d| d.get_temp::<f32>(id));
    if before != Some(offset) {
        ctx.data_mut(|d| d.insert_temp(id, offset));
        if before.is_some() {
            ctx.request_repaint();
        }
    }
    out.content_size.y - out.inner_rect.height() > 0.5
}

/// Keeps the wheel over `rect` from scrolling anything else this pass, vertically: what a body
/// with a scroll of its own does at its ends.
fn contain_wheel_in(ui: &Ui, rect: Rect) {
    if !ui.is_enabled() || !ui.rect_contains_pointer(rect) {
        return;
    }
    let either = ui.style().always_scroll_the_only_direction;
    ui.input_mut(|i| {
        if either {
            i.smooth_scroll_delta.x = 0.0;
        }
        i.smooth_scroll_delta.y = 0.0;
    });
}

/// The sections a stack is given, in order ([`Sections::show`]).
pub struct Stack<'a> {
    sections: Vec<(Section, Body<'a>)>,
}

impl std::fmt::Debug for Stack<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list()
            .entries(self.sections.iter().map(|(s, _)| &s.key))
            .finish()
    }
}

impl<'a> Stack<'a> {
    /// Adds a section, `body` its content (called only while it is open, in a scroll area of
    /// its own).
    pub fn section(&mut self, section: Section, body: impl FnOnce(&mut Ui) + 'a) -> &mut Self {
        self.sections
            .push((section, Body::Scrolled(Box::new(body))));
        self
    }

    /// Adds a section of `count` rows of one height ([`Section::row_height`]) in a scroll area
    /// of its own, windowed: `draw` is called only while it is open, with the rows in view and
    /// the one past them, on every pass, so the window follows the section's size and scroll.
    ///
    /// It draws them from the top of its `ui`, which has no spacing between items, each exactly
    /// that tall (with [`Ui::add_sized`], say), or they drift from their place in the scroll.
    /// Its least height is two rows, or all of them when it has fewer.
    ///
    /// ```
    /// use egui_stack::sections::{Section, Sections, laid_out, row_height};
    ///
    /// # fn main() {
    /// # if cfg!(miri) { return; } // A whole UI: too slow under Miri.
    /// let ctx = egui::Context::default();
    /// let mut out = ctx.run_ui(Default::default(), |ui| {
    ///     let height = row_height(ui);
    ///     Sections::new("log").show(ui, |stack| {
    ///         stack.rows(Section::new("lines", "Lines"), 100_000, move |ui, range| {
    ///             for i in range {
    ///                 let width = ui.available_width();
    ///                 ui.add_sized([width, height], egui::Label::new(format!("Line {i}")));
    ///             }
    ///         });
    ///     });
    /// });
    /// out.textures_delta.clear();
    /// let window = laid_out(&ctx, "log")[0].window.clone().unwrap();
    /// assert!(window.len() < 1_000, "{window:?}");
    /// # }
    /// ```
    pub fn rows(
        &mut self,
        section: Section,
        count: usize,
        draw: impl FnOnce(&mut Ui, Range<usize>) + 'a,
    ) -> &mut Self {
        let draw = Box::new(draw);
        self.sections.push((section, Body::Rows { count, draw }));
        self
    }

    /// Adds a section whose `body` has no scroll area around it (called only while it is
    /// open): for an application's own view that scrolls itself, such as a virtualised list,
    /// which would otherwise nest two scrolls. It fills the body, clipped to it.
    ///
    /// The stack cannot see into that view, so the wheel over a bare body always stops there,
    /// whether or not the view has anywhere to go.
    pub fn bare(&mut self, section: Section, body: impl FnOnce(&mut Ui) + 'a) -> &mut Self {
        self.sections.push((section, Body::Bare(Box::new(body))));
        self
    }
}

/// What the user did to a stack this pass ([`Sections::show`]).
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct SectionsResponse {
    /// A section folded (`false`) or opened (`true`), by key.
    pub toggled: Option<(String, bool)>,
    /// A boundary moved, dragged or from the keyboard.
    pub resized: bool,
    /// A boundary was double-clicked, or Enter pressed on it: every height set by hand dropped.
    pub reset: bool,
    /// The layout after the pass.
    pub layout: Layout,
}

/// Whether a drag pulls: given the modifiers held and the button that drags.
type PullWhen<'a> = Box<dyn Fn(Modifiers, PointerButton) -> bool + 'a>;

/// A stack of collapsible sections filling the room it is given (a panel's height), named for
/// its memory and for [`laid_out`]. See the [module](self) for its behaviour.
pub struct Sections<'a> {
    name: String,
    capitals: bool,
    grab_titles: bool,
    keep_titles: bool,
    band_height: Option<f32>,
    pull_when: PullWhen<'a>,
    overflow_scroll: Option<ScrollStyle>,
    fill: bool,
    bars: Bars,
    painter: &'a (dyn DockPainter + Sync),
    labels: Labels,
}

impl std::fmt::Debug for Sections<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sections")
            .field("name", &self.name)
            .field("capitals", &self.capitals)
            .field("grab_titles", &self.grab_titles)
            .field("keep_titles", &self.keep_titles)
            .field("band_height", &self.band_height)
            .field("fill", &self.fill)
            .field("labels", &self.labels)
            .finish_non_exhaustive()
    }
}

impl<'a> Sections<'a> {
    /// A stack named `name`, unique in the application: its layout is remembered under it and
    /// [`laid_out`] reads it.
    pub fn new(name: impl Into<String>) -> Self {
        Sections {
            name: name.into(),
            capitals: true,
            grab_titles: false,
            keep_titles: false,
            band_height: None,
            pull_when: pull_with(Some(Modifiers::COMMAND)),
            overflow_scroll: None,
            fill: true,
            bars: Bars::default(),
            painter: &StylePainter,
            labels: Labels::ENGLISH,
        }
    }

    /// The painter of the bands and boundaries, [`StylePainter`] by default
    /// ([`DockPainter::section_band`], [`DockPainter::section_boundary`]).
    pub fn painter(mut self, painter: &'a (dyn DockPainter + Sync)) -> Self {
        self.painter = painter;
        self
    }

    /// The accessible names, English by default: a boundary is named from
    /// [`Labels::resize_section`].
    pub fn labels(mut self, labels: Labels) -> Self {
        self.labels = labels;
        self
    }

    /// Whether titles are shown in capitals (yes by default, as a code editor's side bar does);
    /// their accessible names keep their case.
    pub fn capitals(mut self, capitals: bool) -> Self {
        self.capitals = capitals;
        self
    }

    /// The height of a band, in points; by default a row's ([`row_height`]) and 4 more.
    pub fn band_height(mut self, points: f32) -> Self {
        self.band_height = Some(points.max(1.0));
        self
    }

    /// The modifiers that make a primary drag on a boundary, or Up and Down on a focused one,
    /// pull rather than push ([`DragMode::Pull`]): Ctrl by default, Cmd on macOS
    /// ([`Modifiers::COMMAND`]). They must be held exactly; `None` turns the pull off.
    pub fn pull_modifiers(mut self, modifiers: Option<Modifiers>) -> Self {
        self.pull_when = pull_with(modifiers);
        self
    }

    /// Decides itself whether a drag pulls, given the modifiers held and the button that
    /// drags, for an application whose pointer gestures can be rebound; Up and Down on a
    /// focused boundary ask it as for the primary button. Replaces
    /// [`pull_modifiers`](Self::pull_modifiers).
    pub fn pull_when(mut self, pulls: impl Fn(Modifiers, PointerButton) -> bool + 'a) -> Self {
        self.pull_when = Box::new(pulls);
        self
    }

    /// The style of the stack's own scroll bar, shown only when even the least heights do not
    /// fit; by default the `Ui`'s. The bodies keep the `Ui`'s.
    pub fn overflow_scroll(mut self, style: ScrollStyle) -> Self {
        self.overflow_scroll = Some(style);
        self
    }

    /// Whether the stack fills the room it is given (yes by default): the last open section
    /// takes what the others leave, so the bands of folded sections below it sit at the
    /// bottom.
    ///
    /// Off, the stack is sized by its content: an open section sized by the list is never laid
    /// out taller than what its body laid out on the last pass (its rows, its content, or what
    /// a bare body drew), the long ones sharing what is left, and the folded bands follow right
    /// under the last open section, the room left empty under the last band
    /// ([`plan::fit_content`]). A section set by hand keeps its height, the last open one too:
    /// a drag down grows the section above past its content, into the room under the last
    /// band, and a drag up shrinks it to its least height. The content only places the
    /// sections until a drag; the least heights and the scrolling inside a section are as when
    /// it fills.
    pub fn fill(mut self, fill: bool) -> Self {
        self.fill = fill;
        self
    }

    /// The style the scroll bars of the bodies and of the stack are drawn in: egui paints a
    /// bar from the visuals of the `Ui` its scroll area is shown in (the thumb in the widget
    /// states' colours), so an application giving its bars colours of their own passes the
    /// style it shows its scroll areas in. The content keeps the `Ui`'s style, and the
    /// spacing (the bars' sizes included) stays the `Ui`'s. By default, the `Ui`'s.
    pub fn bar_style(mut self, style: Arc<egui::Style>) -> Self {
        self.bars.style = Some(style);
        self
    }

    /// Points kept free at both ends of each scroll bar's track, along it, in the bodies and
    /// the stack: the thumb starts and ends that far from the area's ends
    /// (`ScrollArea::scroll_bar_rect`). 0 by default.
    pub fn bar_track_inset(mut self, points: f32) -> Self {
        self.bars.inset = if points.is_finite() {
            points.max(0.0)
        } else {
            0.0
        };
        self
    }

    /// Whether a title drags as well as folds (no by default, as the line above a band is what
    /// classic apps resize by, and a code editor's headers reorder when dragged).
    ///
    /// A press that moves is a drag, by the same rules as a boundary's, the title staying under
    /// the pointer; one that does not is a fold. A title under a folded section resizes the
    /// first open one above, and with nothing open above it does not move. A double click drops
    /// the heights set by hand; as its first click has already folded, the section is seen
    /// folded until the second. A band keeps its name and its role.
    pub fn grab_titles(mut self, grab: bool) -> Self {
        self.grab_titles = grab;
        self
    }

    /// Whether a title stays where it is when its section folds or opens (no by default, as a
    /// code editor's panes divide the room again, and the title moves).
    ///
    /// Folding, the open sections above keep their shown height, as if set by hand; those below
    /// move up, and with nothing below the room stays empty. Opening, the section takes its room
    /// below its title, from the sections below, then from the empty room. In a panel with no
    /// room for it, those above still give way, and the title moves. A double click gets the
    /// shares back.
    pub fn keep_titles(mut self, keep: bool) -> Self {
        self.keep_titles = keep;
        self
    }

    /// Lays the sections `add` gives out in the room left in `ui`.
    pub fn show<'b>(self, ui: &mut Ui, add: impl FnOnce(&mut Stack<'b>)) -> SectionsResponse {
        let mut stack = Stack {
            sections: Vec::new(),
        };
        add(&mut stack);
        self.lay_out(ui, stack.sections)
    }

    fn lay_out(&self, ui: &mut Ui, sections: Vec<(Section, Body<'_>)>) -> SectionsResponse {
        let ctx = ui.ctx().clone();
        let row = row_height(ui);
        let band_h = self.band_height.unwrap_or(row + 4.0);

        let mut layout = Layout::load(&ctx, &self.name).unwrap_or_default();
        for (s, _) in &sections {
            layout.sections.entry(s.key.clone()).or_insert(Remembered {
                open: s.open,
                height: None,
            });
        }
        let room = ui.available_rect_before_wrap();
        let finite = room.height().is_finite();
        // Each section's greatest height, never under its least (infinite: no bound).
        let mut bounds: Vec<f32> = Vec::with_capacity(sections.len());
        let asked: Vec<Asked> = sections
            .iter()
            .map(|(s, body)| {
                let remembered = layout.sections[&s.key];
                let least = s
                    .min_height
                    .unwrap_or_else(|| body.least(ui, s.row_height.unwrap_or(row)));
                let bound = s
                    .max_height
                    .map_or(f32::INFINITY, |m| m.points(room.height()).max(least));
                bounds.push(bound);
                Asked {
                    open: remembered.open,
                    least,
                    // A height set by hand past the bound (kept from before it) is the bound.
                    hand: remembered
                        .height
                        .filter(|h| h.is_finite())
                        .map(|h| h.min(bound)),
                }
            })
            .collect();
        let keys: Vec<String> = sections.iter().map(|(s, _)| s.key.clone()).collect();
        let open: Vec<bool> = asked.iter().map(|a| a.open).collect();

        let bands = sections.len() as f32 * band_h;
        let space = if finite {
            (room.height() - bands).max(0.0)
        } else {
            0.0
        };
        // Sized by the content: each section's content height on the last pass.
        let content: BTreeMap<String, f32> =
            get_temp(&ctx, &self.name, CONTENT).unwrap_or_default();
        // The most each section sized by the list takes: its bound, and in a stack sized by its
        // content, its content too.
        let most: Vec<f32> = if self.fill {
            if bounds.iter().all(|b| b.is_infinite()) {
                Vec::new()
            } else {
                bounds.clone()
            }
        } else {
            keys.iter()
                .zip(&bounds)
                .map(|(k, &bound)| content.get(k).copied().unwrap_or(f32::INFINITY).min(bound))
                .collect()
        };
        let (fit, spare) = if self.fill {
            plan::fit_within(&asked, &most, space, layout.empty)
        } else {
            plan::fit_content(&asked, &most, space, layout.empty)
        };
        let tall = bands + fit.heights.iter().sum::<f32>();
        let overflows = finite && tall - room.height() > 0.5;
        let stack = Shown {
            keys: &keys,
            asked: &asked,
            heights: &fit.heights,
            most: &most,
            bounds: &bounds,
            space,
            // What the content leaves is not the layout's: a drag keeps the empty room asked.
            empty: fit.empty - spare,
            // Sized by the content, a drag takes the room the content leaves too: the section
            // above grows past its content.
            free: if self.fill {
                fit.empty - spare
            } else {
                fit.empty
            },
            fill: self.fill,
            step: row,
        };
        let mut contents: BTreeMap<String, f32> = BTreeMap::new();

        let mut response = SectionsResponse::default();
        let mut laid: Vec<Laid> = Vec::with_capacity(sections.len());
        let base = memory_id(&self.name);
        let reach = ui.style().interaction.resize_grab_radius_side;
        let window = ctx.options(|o| o.input_options.max_double_click_delay);
        let (now, released) =
            ui.input(|i| (i.time, i.pointer.any_down() || i.pointer.any_released()));
        let last_click: Option<TitleClick> = get_temp(&ctx, &self.name, TITLE_CLICK);
        let draw = |ui: &mut Ui| {
            let left = ui.max_rect().left();
            let width = ui.available_width();
            let top = ui.cursor().min.y;
            let mut y = top;
            let mut boundaries = Vec::new();
            for (i, (section, body)) in sections.into_iter().enumerate() {
                let is_open = asked[i].open;
                let band = Rect::from_min_size(pos2(left, y), vec2(width, band_h));
                let mut hit = band;
                // A boundary is registered between the section above it and the band under it,
                // so Tab reaches it in reading order; the band leaves it the room it would
                // otherwise win under the pointer. Under a folded section it resizes the first
                // open one above, as the line above that folded section does, which alone takes
                // the focus: one stop per pair of open sections.
                let above = i
                    .checked_sub(1)
                    .and_then(|line| plan::resized_above(&open, line));
                let room_below = fit.empty > 0.5 || asked[i..].iter().any(|a| a.open);
                // A title that drags keeps the whole band, or a press on its top would drag but
                // never fold: the line is on the section's side alone, and under a folded
                // section, whose title does the same, there is none.
                let line = above
                    .filter(|_| room_below)
                    .filter(|&above| !self.grab_titles || above + 1 == i);
                if let Some(above) = line {
                    let edge = if self.grab_titles {
                        Rect::from_x_y_ranges(band.x_range(), y - reach..=y)
                    } else {
                        hit.min.y = (hit.min.y + reach).min(hit.max.y);
                        Rect::from_x_y_ranges(band.x_range(), y - reach..=y + reach)
                    };
                    let id = base.with(("boundary", &laid[i - 1].key));
                    let sense = if above == i - 1 {
                        Sense::click_and_drag()
                    } else {
                        Sense::CLICK | Sense::DRAG
                    };
                    let r = ui.interact(edge, id, sense);
                    let name = self
                        .labels
                        .resize_section
                        .replace("{section}", &laid[above].title);
                    name_boundary(&ctx, &r, &name, &stack, above);
                    if overflows && r.gained_focus() {
                        r.scroll_to_me(None);
                    }
                    let mode =
                        self.work_boundary(ui, &r, &stack, above, &mut layout, &mut response);
                    laid[above].drag = mode.or(laid[above].drag);
                    boundaries.push((edge, y, r.id, part_state(&r)));
                }
                let grabbed = above.filter(|_| self.grab_titles);
                let sense = if grabbed.is_some() {
                    Sense::click_and_drag()
                } else {
                    Sense::click()
                };
                let r = ui.interact(hit, base.with(("band", &section.key)), sense);
                if let Some(above) = grabbed {
                    let mode = self.work_drag(ui, &r, &stack, above, &mut layout, &mut response);
                    laid[above].drag = mode.or(laid[above].drag);
                }
                self.paint_band(ui, &section, band, &r, is_open);
                name_band(&ctx, &r, &section.name(), is_open);
                if overflows && r.gained_focus() {
                    r.scroll_to_me(None);
                }
                if self.grab_titles && r.double_clicked() {
                    // Its first click folded: the second puts that back. Both in one pass, they
                    // cancel out.
                    if last_click.as_ref().is_some_and(|c| c.key == section.key) {
                        stack.fold(&mut layout, &mut response, i, false);
                    }
                    self.reset(&ctx, &mut layout, &mut response);
                } else if r.clicked() || arrow_folds(ui, &r, is_open) {
                    stack.fold(&mut layout, &mut response, i, self.keep_titles);
                    if self.grab_titles && r.clicked() {
                        let key = section.key.clone();
                        let click = TitleClick { key, band, at: now };
                        set_temp(&ctx, &self.name, TITLE_CLICK, click);
                        ctx.request_repaint_after_secs(window as f32);
                    }
                }
                y = band.bottom();
                let mut body_rect = None;
                let mut drawn = None;
                if is_open {
                    let rect = Rect::from_min_size(pos2(left, y), vec2(width, fit.heights[i]));
                    let mut child = ui.new_child(
                        UiBuilder::new()
                            .id_salt(("egui_stack.sections.body", &self.name, &section.key))
                            .max_rect(rect)
                            .layout(egui::Layout::top_down(egui::Align::Min)),
                    );
                    child.set_clip_rect(rect.intersect(ui.clip_rect()));
                    let shown = body.draw(
                        &mut child,
                        &section.key,
                        section.row_height.unwrap_or(row),
                        &self.bars,
                    );
                    contents.insert(section.key.clone(), shown.content);
                    if shown.holds_wheel {
                        contain_wheel_in(&child, rect);
                    }
                    drawn = Some(shown);
                    y = rect.bottom();
                    body_rect = Some(rect);
                }
                laid.push(Laid {
                    key: section.key,
                    title: section.title,
                    count: section.count,
                    open: is_open,
                    height: fit.heights[i],
                    by_hand: fit.by_hand[i],
                    band,
                    body: body_rect,
                    scroll: drawn.as_ref().map_or(0.0, |d| d.scroll),
                    window: drawn.and_then(|d| d.window),
                    empty: if i + 1 == asked.len() { fit.empty } else { 0.0 },
                    drag: None,
                });
            }
            // The fold may have taken the title from under the pointer: what moved there must
            // not take the second click of a double click, nor the title lose it. Drawn last,
            // so it has the pointer.
            let moved = last_click
                .as_ref()
                .filter(|c| now - c.at <= window || released)
                .and_then(|c| {
                    let at = stack.keys.iter().position(|k| *k == c.key)?;
                    ((laid[at].band.top() - c.band.top()).abs() > 0.5).then_some((at, c.band))
                });
            if let Some((at, band)) = moved {
                let r = ui.interact(band, base.with("title_click"), Sense::click());
                if r.double_clicked() {
                    stack.fold(&mut layout, &mut response, at, false);
                    self.reset(&ctx, &mut layout, &mut response);
                } else if r.clicked() {
                    stack.fold(&mut layout, &mut response, at, self.keep_titles);
                }
            }
            // Painted last: the band under a boundary would cover its ring.
            for (rect, line, id, state) in boundaries {
                if ui.is_rect_visible(rect) {
                    let boundary = SectionBoundaryPaint {
                        rect,
                        line,
                        id,
                        state,
                        style: ui.style(),
                    };
                    self.painter.section_boundary(ui.painter(), &boundary);
                }
            }
            let end = if finite && !overflows {
                y.min(top + room.height())
            } else {
                y
            };
            ui.advance_cursor_after_rect(Rect::from_min_max(
                pos2(left, top),
                pos2(left + width, end.max(top)),
            ));
        };
        if finite {
            // Always in this area, overflowing or not, so that a body's own scroll keeps its
            // place when the panel crosses the line.
            let area = egui::ScrollArea::vertical()
                .id_salt(("egui_stack.sections.stack", &self.name))
                .auto_shrink([false, false])
                .max_height(room.height())
                .min_scrolled_height(0.0)
                .scroll_source(ScrollSource {
                    drag: DragScroll::Never,
                    ..ScrollSource::default()
                })
                .scroll_bar_visibility(if overflows {
                    ScrollBarVisibility::AlwaysVisible
                } else {
                    ScrollBarVisibility::AlwaysHidden
                });
            let area = self.bars.on(area, room);
            // The stack's bar may have a style of its own; the bodies keep the Ui's.
            let bodies = ui.spacing().scroll;
            let content = ui.style().clone();
            ui.scope(|ui| {
                if overflows && let Some(style) = self.overflow_scroll {
                    ui.spacing_mut().scroll = style;
                }
                self.bars.show(ui, |ui| {
                    area.show(ui, |ui| {
                        ui.set_style(content);
                        ui.spacing_mut().scroll = bodies;
                        draw(ui);
                    })
                });
            });
        } else {
            draw(ui);
        }
        if !self.fill && contents != content {
            // The heights follow the content from the next pass.
            ctx.request_repaint();
        }
        set_temp(&ctx, &self.name, CONTENT, contents);

        if response.toggled.is_some() || response.resized || response.reset {
            ctx.request_repaint();
        }
        layout.store(&ctx, &self.name);
        set_temp(&ctx, &self.name, LAID, laid);
        response.layout = layout;
        response
    }

    /// Drops the heights set by hand and the empty room.
    fn reset(&self, ctx: &Context, layout: &mut Layout, response: &mut SectionsResponse) {
        layout.reset_heights();
        response.reset = true;
        remove_temp::<Drag>(ctx, &self.name, DRAG);
        remove_temp::<TitleClick>(ctx, &self.name, TITLE_CLICK);
    }

    /// Paints a band through the painter.
    fn paint_band(&self, ui: &Ui, section: &Section, band: Rect, r: &Response, open: bool) {
        if !ui.is_rect_visible(band) {
            return;
        }
        let title = if self.capitals {
            section.title.to_uppercase()
        } else {
            section.title.clone()
        };
        let paint = SectionBandPaint {
            rect: band,
            id: r.id,
            title: &title,
            count: section.count,
            open,
            state: part_state(r),
            style: ui.style(),
        };
        self.painter.section_band(ui.painter(), &paint);
    }

    /// Whether these keys and this button make a drag pull.
    fn pulls(&self, modifiers: Modifiers, button: PointerButton) -> bool {
        (self.pull_when)(modifiers, button)
    }

    /// How a drag under way moves the stack, by the keys held now and the button that holds it.
    fn drag_mode(&self, ui: &Ui) -> DragMode {
        let pull = ui.input(|i| {
            [
                PointerButton::Primary,
                PointerButton::Secondary,
                PointerButton::Middle,
                PointerButton::Extra1,
                PointerButton::Extra2,
            ]
            .into_iter()
            .any(|b| i.pointer.button_down(b) && self.pulls(i.modifiers, b))
        });
        if pull { DragMode::Pull } else { DragMode::Push }
    }

    /// How the arrows on a boundary move the stack: as a primary drag with the same keys held
    /// would.
    fn keys_mode(&self, ui: &Ui) -> DragMode {
        if ui.input(|i| self.pulls(i.modifiers, PointerButton::Primary)) {
            DragMode::Pull
        } else {
            DragMode::Push
        }
    }

    /// What a pointer dragging `r` does to the section `above`, a boundary or a title alike.
    /// Says how it moves the stack, while it does.
    ///
    /// Each pass moves the boundary from where the stack shows it to where the pointer wants it,
    /// as a fresh drag would: what the drag shrank stays shrunk, and past a stop nothing moves
    /// until the pointer is back on the boundary. The keys pressed or released during the drag
    /// change only how the next moves give way; no move goes further than the pointer did since
    /// the last pass, so nothing jumps.
    fn work_drag(
        &self,
        ui: &Ui,
        r: &Response,
        stack: &Shown<'_>,
        above: usize,
        layout: &mut Layout,
        response: &mut SectionsResponse,
    ) -> Option<DragMode> {
        let ctx = ui.ctx();
        let name = &self.name;
        if r.drag_started() {
            let origin = ui
                .input(|i| i.pointer.press_origin())
                .map_or(r.rect.center().y, |p| p.y);
            let drag = Drag {
                origin,
                top: stack.top(above),
                hands: stack.hands(),
                last: origin,
            };
            set_temp(ctx, name, DRAG, drag);
        }
        let mut mode = None;
        if r.dragged() {
            ctx.set_cursor_icon(CursorIcon::ResizeVertical);
            let drag =
                get_temp::<Drag>(ctx, name, DRAG).filter(|d| d.hands.0.len() == stack.keys.len());
            if let (Some(mut drag), Some(pointer)) = (drag, r.interact_pointer_pos()) {
                let now = self.drag_mode(ui);
                let wanted = pointer.y - drag.origin;
                // A boundary stopped short of the pointer in one mode may go further in the
                // other: it catches up as the pointer moves, never faster, so that nothing jumps
                // when the keys change.
                let reach = (pointer.y - drag.last).abs();
                let delta = (wanted - (stack.top(above) - drag.top)).clamp(-reach, reach);
                if delta.abs() > 0.01 {
                    response.resized |= stack.drag(layout, &drag.hands, above, delta, now);
                }
                drag.last = pointer.y;
                set_temp(ctx, name, DRAG, drag);
                mode = Some(now);
            }
        }
        if r.drag_stopped() {
            remove_temp::<Drag>(ctx, name, DRAG);
        }
        mode
    }

    /// What the user does to the boundary under `above`: the pointer, the arrow keys, Enter and
    /// assistive technology's steps. Says how a drag moves the stack, while one does.
    fn work_boundary(
        &self,
        ui: &Ui,
        r: &Response,
        stack: &Shown<'_>,
        above: usize,
        layout: &mut Layout,
        response: &mut SectionsResponse,
    ) -> Option<DragMode> {
        let ctx = ui.ctx();
        if r.hovered() {
            ctx.set_cursor_icon(CursorIcon::ResizeVertical);
        }
        if r.has_focus() {
            // egui moves the focus with an arrow key unless the widget says it uses them.
            ui.memory_mut(|m| {
                m.set_focus_lock_filter(
                    r.id,
                    EventFilter {
                        vertical_arrows: true,
                        ..Default::default()
                    },
                );
            });
        }
        if r.double_clicked() || (r.has_focus() && ui.input(|i| i.key_pressed(Key::Enter))) {
            layout.reset_heights();
            response.reset = true;
            remove_temp::<Drag>(ctx, &self.name, DRAG);
            return None;
        }
        let dragging = self.work_drag(ui, r, stack, above, layout, response);
        let focused = r.has_focus();
        let steps = ui.input(|i| {
            let keys = if focused {
                i.num_presses(Key::ArrowDown) as f32 - i.num_presses(Key::ArrowUp) as f32
            } else {
                0.0
            };
            keys + i.num_accesskit_action_requests(r.id, Action::Increment) as f32
                - i.num_accesskit_action_requests(r.id, Action::Decrement) as f32
        });
        if steps != 0.0 {
            let delta = steps * stack.step;
            let mode = self.keys_mode(ui);
            response.resized |= stack.drag(layout, &stack.hands(), above, delta, mode);
        }
        dragging
    }
}

/// The rule of [`Sections::pull_modifiers`]: a primary drag with exactly these modifiers.
fn pull_with<'a>(modifiers: Option<Modifiers>) -> PullWhen<'a> {
    Box::new(move |held: Modifiers, button| {
        button == PointerButton::Primary && modifiers.is_some_and(|m| held.matches_exact(m))
    })
}

/// A part's state for the painter.
fn part_state(r: &Response) -> PartState {
    PartState::NONE
        .with(PartState::HOVERED, r.hovered())
        .with(PartState::PRESSED, r.is_pointer_button_down_on())
        .with(PartState::FOCUSED, r.has_focus())
        .with(PartState::DRAGGED, r.dragged())
}

/// Names a band for assistive technology: a button, expanded or not.
fn name_band(ctx: &Context, r: &Response, name: &str, open: bool) {
    let enabled = r.enabled();
    r.widget_info(|| WidgetInfo::labeled(Role::Button, enabled, name));
    ctx.accesskit_node_builder(r.id, |node| node.set_expanded(open));
}

/// The stack as this pass shows it: what a boundary works from.
struct Shown<'a> {
    keys: &'a [String],
    asked: &'a [Asked],
    heights: &'a [f32],
    /// What each section's content asks, in a stack sized by its content ([`plan::fit_within`]).
    most: &'a [f32],
    /// Each section's greatest height ([`Section::max_height`]; infinite: no bound).
    bounds: &'a [f32],
    /// The height the open sections divide.
    space: f32,
    /// The empty room kept under the last section, as the layout asks it.
    empty: f32,
    /// The empty room a drag can take: the room kept, and in a stack sized by its content, the
    /// room the content leaves.
    free: f32,
    /// Whether the stack fills its room ([`Sections::fill`]).
    fill: bool,
    /// How far an arrow key moves a boundary.
    step: f32,
}

impl Shown<'_> {
    fn placed(&self, heights: &[f32]) -> Vec<Placed> {
        self.asked
            .iter()
            .zip(heights)
            .map(|(a, &height)| Placed {
                open: a.open,
                least: a.least,
                height,
            })
            .collect()
    }

    /// The boundary under `above` moved `delta` from where the stack shows it, in `mode`,
    /// pinned against `hands`. Says whether this changes what is shown.
    fn drag(
        &self,
        layout: &mut Layout,
        hands: &Hands,
        above: usize,
        delta: f32,
        mode: DragMode,
    ) -> bool {
        let placed = self.placed(self.heights);
        let delta = self.bounded(above, delta, mode);
        let plan = plan::plan_drag(&placed, above, delta, self.free, mode);
        let pinned = if self.fill {
            plan::pin_plan_within(
                &hands.asked(self),
                self.most,
                self.space,
                plan.empty,
                &plan.heights,
            )
        } else {
            // Every open section the drag moves keeps its height by hand, the last one too: the
            // room under the last band is what they leave, none to keep.
            plan::pin_plan_content(
                &hands.asked(self),
                self.most,
                self.space,
                0.0,
                &plan.heights,
            )
        };
        for (key, hand) in self.keys.iter().zip(pinned) {
            layout.set_height(key, hand);
        }
        layout.empty = if self.fill { plan.empty } else { 0.0 };
        let moved = |(a, b): (&f32, &f32)| (a - b).abs() > 0.01;
        (plan.empty - self.free).abs() > 0.01 || plan.heights.iter().zip(self.heights).any(moved)
    }

    /// `delta` on the boundary under `above`, cut so that no section grows past its bound:
    /// down, the section above grows; up and pushing, the first open one below does.
    fn bounded(&self, above: usize, delta: f32, mode: DragMode) -> f32 {
        let room = |i: usize| (self.bounds[i] - self.heights[i]).max(0.0);
        if delta > 0.0 {
            delta.min(room(above))
        } else if delta < 0.0 && mode == DragMode::Push {
            let below = (above + 1..self.asked.len()).find(|&i| self.asked[i].open);
            below.map_or(delta, |i| delta.max(-room(i)))
        } else {
            delta
        }
    }

    /// The heights kept by hand as shown: what a drag or a key press pins its plans against.
    fn hands(&self) -> Hands {
        Hands(self.asked.iter().map(|a| a.hand).collect())
    }

    /// How far down the boundary under `above` is: the heights above it, summed (the folded
    /// bands between do not change).
    fn top(&self, above: usize) -> f32 {
        self.heights[..=above].iter().sum()
    }

    /// Folds or opens `section`, its title staying where it is if `keep`.
    fn fold(
        &self,
        layout: &mut Layout,
        response: &mut SectionsResponse,
        section: usize,
        keep: bool,
    ) {
        if keep {
            let kept = plan::keep_title(self.asked, self.heights, self.empty, section);
            for (key, hand) in self.keys.iter().zip(kept.hands) {
                layout.set_height(key, hand);
            }
            layout.empty = kept.empty;
        }
        let (key, open) = (&self.keys[section], !self.asked[section].open);
        layout.set_open(key, open);
        response.toggled = Some((key.clone(), open));
    }

    /// What the boundary under `above` can do from here, up and down.
    fn reach(&self, above: usize) -> (bool, f32) {
        let placed = self.placed(self.heights);
        let plan = |delta| plan::plan_drag(&placed, above, delta, self.free, DragMode::Push);
        (
            self.bounded(above, -self.step, DragMode::Push) < 0.0 && plan(-self.step).moved < 0.0,
            plan(self.space).heights[above].min(self.bounds[above]),
        )
    }
}

/// Names a boundary for assistive technology: a splitter, its value the height of the section
/// above it, with the actions the arrows have.
fn name_boundary(ctx: &Context, r: &Response, name: &str, stack: &Shown<'_>, above: usize) {
    let value = stack.heights[above];
    let (shrinks, most) = stack.reach(above);
    r.widget_info(|| WidgetInfo::labeled(Role::Unknown, true, name));
    ctx.accesskit_node_builder(r.id, |node| {
        node.set_role(Role::Splitter);
        node.set_numeric_value(f64::from(value));
        node.set_min_numeric_value(f64::from(stack.asked[above].least.min(value)));
        node.set_max_numeric_value(f64::from(most));
        node.set_numeric_value_step(f64::from(stack.step));
        if most > value {
            node.add_action(Action::Increment);
        }
        if shrinks {
            node.add_action(Action::Decrement);
        }
    });
}

/// Left folds a band with the keyboard and Right opens it, as a code editor's headers do. Says
/// whether the band changes.
fn arrow_folds(ui: &Ui, r: &Response, open: bool) -> bool {
    if !r.has_focus() {
        return false;
    }
    // As for a boundary, the arrows would move the focus.
    ui.memory_mut(|m| {
        m.set_focus_lock_filter(
            r.id,
            EventFilter {
                horizontal_arrows: true,
                ..Default::default()
            },
        );
    });
    let key = if open {
        Key::ArrowLeft
    } else {
        Key::ArrowRight
    };
    ui.input(|i| i.key_pressed(key))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(feature = "serde")]
    fn a_layout_keeps_its_heights_through_a_round_trip_and_drops_them_on_reset() {
        let mut layout = Layout::default();
        layout.set_open("live", false);
        layout.set_height("live", Some(120.0));
        layout.set_height("idle", Some(40.0));
        let text = serde_json::to_string(&layout).unwrap();
        let mut back: Layout = serde_json::from_str(&text).unwrap();
        assert_eq!(back, layout);
        back.reset_heights();
        assert_eq!(back.height("live"), None);
        assert_eq!(back.is_open("live"), Some(false), "the fold stays");
    }

    #[test]
    #[cfg(feature = "serde")]
    fn a_layout_without_heights_or_empty_room_reads_as_sized_by_the_list() {
        let old = r#"{"sections":{"live":{"open":false},"idle":{"open":true}}}"#;
        let layout: Layout = serde_json::from_str(old).unwrap();
        assert_eq!(layout.is_open("live"), Some(false));
        assert_eq!(layout.height("idle"), None);
        assert_eq!(layout.empty, 0.0);
    }

    #[test]
    fn a_band_count_past_99_is_capped() {
        let style = egui::Style::default();
        let band = |count| SectionBandPaint {
            rect: Rect::ZERO,
            id: Id::NULL,
            title: "",
            count,
            open: true,
            state: PartState::NONE,
            style: &style,
        };
        assert_eq!(band(Some(3)).count_text().as_deref(), Some("3"));
        assert_eq!(band(Some(99)).count_text().as_deref(), Some("99"));
        assert_eq!(band(Some(100)).count_text().as_deref(), Some("99+"));
        assert_eq!(band(None).count_text(), None);
    }
}
