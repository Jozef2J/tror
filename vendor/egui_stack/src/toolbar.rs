//! A toolbar row: a row (or a column) of tools the caller gives, buttons with an icon, a label
//! or both, as in an office suite's or a code editor's toolbars and an image editor's palette.
//!
//! - A tool is a button ([`Tool::button`]), a toggle ([`Tool::toggle`], or a radio choice,
//!   [`Tool::radio`]), a menu ([`Tool::menu`], a button opening the caller's menu), a separator
//!   between groups, a spacer that pushes the tools after it to the end, or a widget of the
//!   caller's own ([`Tool::custom`]).
//! - The row says what was clicked ([`ToolRowResponse::clicked`]): the caller runs it. The row
//!   holds no state of the tools: a toggle shows the state it is given.
//! - [`ToolDisplay`] shows icons alone (the label then in the tooltip), icons and labels, or
//!   labels alone; a tool without an icon shows its label.
//! - The tools that do not fit go, from the end, into a **More** button opening them as a
//!   menu, in their order; a separator never ends the row nor starts that menu, and a spacer
//!   gives its room first.
//! - **Named**: the row is a [`Role::Toolbar`] node, named by [`ToolRow::title`]; each tool is
//!   named by its label, a toggle announced as toggled, a radio choice as checked, a busy tool
//!   as busy; the More button is named "More tools" ([`Labels::more_tools`]), or "More tools:
//!   Formatting" in a row with a title.
//! - **The keyboard** ([`TabFocus::Roving`], the default, as in WAI-ARIA's toolbar): the row is
//!   one Tab stop, the tool focused there last (the first the first time). Left and Right (Up
//!   and Down in a column) go to the tool before or after, round from one end to the other;
//!   Home and End to the first and the last; a disabled tool is passed over, and a caller's own
//!   widget keeps its own place in egui's order. Enter or Space clicks. [`TabFocus::Each`]
//!   makes every tool a Tab stop and leaves the arrows to egui.
//!
//! The tools, the More button and the separators are painted by a [`DockPainter`]
//! ([`DockPainter::tool_item`], [`DockPainter::tool_more`], [`DockPainter::tool_separator`]),
//! [`StylePainter`] by default, from the `Ui`'s [`egui::Style`].
//!
//! [`laid_out`] gives the row as drawn on the last pass, every tool with its state, whether it
//! went into the More menu and its widget id, so that a test or an agent reads the row rather
//! than pixels. [`focused`] and [`focus`] read and move the keyboard focus in it.
//!
//! ```
//! use egui_stack::toolbar::{Tool, ToolRow, laid_out};
//!
//! # fn main() {
//! # if cfg!(miri) { return; } // A whole UI: too slow under Miri.
//! let ctx = egui::Context::default();
//! let mut bold = false;
//! let mut out = ctx.run_ui(Default::default(), |ui| {
//!     let row = ToolRow::new("format").title("Formatting").show(
//!         ui,
//!         [
//!             Tool::button("cut", "Cut"),
//!             Tool::button("copy", "Copy"),
//!             Tool::separator(),
//!             Tool::toggle("bold", "Bold", bold),
//!         ],
//!     );
//!     if row.clicked.as_deref() == Some("bold") {
//!         bold = !bold;
//!     }
//! });
//! out.textures_delta.clear();
//! let row = laid_out(&ctx, "format").unwrap();
//! assert_eq!(row.tools.len(), 4);
//! assert!(row.tools.iter().all(|t| !t.overflow));
//! # }
//! ```

use std::sync::Arc;

use egui::accesskit::{self, Role};
use egui::containers::menu::MenuConfig;
use egui::load::TexturePoll;
use egui::{
    Color32, Context, EventFilter, Galley, Id, Image, Key, Painter, Popup, Pos2, Rect, Response,
    Sense, TextStyle, Ui, UiBuilder, UiKind, UiStackInfo, Vec2, WidgetInfo, pos2, vec2,
};

use crate::{
    DockPainter, Labels, PartState, StylePainter, TabFocus, ToolItemPaint, ToolMorePaint,
    ToolSeparatorPaint,
};

/// Which way a [`ToolRow`] runs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub enum Orientation {
    /// A row, from the left.
    #[default]
    Horizontal,
    /// A column, from the top (a tool palette).
    Vertical,
}

/// What the tools of a [`ToolRow`] show.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub enum ToolDisplay {
    /// Icons alone, the label in the tooltip; a tool without an icon shows its label.
    #[default]
    Icons,
    /// Each icon with its label beside it.
    Beside,
    /// Labels alone.
    Text,
}

/// What a tool is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum ToolKind {
    /// A button ([`Tool::button`]).
    Button,
    /// A button on or off ([`Tool::toggle`]).
    Toggle,
    /// A choice among others ([`Tool::radio`]).
    Radio,
    /// A button opening a menu ([`Tool::menu`]).
    Menu,
    /// The line between two groups ([`Tool::separator`]).
    Separator,
    /// Room that pushes the tools after it to the end ([`Tool::spacer`]).
    Spacer,
    /// A widget of the caller's own ([`Tool::custom`]).
    Custom,
}

impl ToolKind {
    /// Whether it is a button the row draws, clicks and moves the focus over.
    fn is_button(self) -> bool {
        matches!(self, Self::Button | Self::Toggle | Self::Radio | Self::Menu)
    }
}

/// Paints an icon in a square, in a colour.
type PaintIcon<'a> = Box<dyn Fn(&Painter, Rect, Color32) + 'a>;

/// A tool's icon: an image, or a closure painting it.
pub enum ToolIcon<'a> {
    /// An image, painted in its own colours (its [`Image::tint`] kept), faded when the tool is
    /// disabled.
    Image(Image<'a>),
    /// A closure painting the icon in the square it is given, in the colour it is given (the
    /// tool's text colour for its state): for icons drawn as shapes or from a font.
    Paint(PaintIcon<'a>),
}

impl std::fmt::Debug for ToolIcon<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Image(image) => f.debug_tuple("Image").field(image).finish(),
            Self::Paint(_) => f.write_str("Paint(..)"),
        }
    }
}

impl<'a> From<Image<'a>> for ToolIcon<'a> {
    fn from(image: Image<'a>) -> Self {
        Self::Image(image)
    }
}

impl ToolIcon<'_> {
    /// Paints the icon in `rect`, in `color` for a closure; an image keeps its colours and is
    /// faded when not `enabled`. An image still loading paints nothing yet.
    pub fn paint(&self, painter: &Painter, rect: Rect, color: Color32, enabled: bool) {
        match self {
            Self::Paint(paint) => paint(painter, rect, color),
            Self::Image(image) => {
                if let Ok(TexturePoll::Ready { texture }) =
                    image.load_for_size(painter.ctx(), rect.size())
                {
                    let mut options = image.image_options().clone();
                    if !enabled {
                        options.tint = options.tint.gamma_multiply(0.4);
                    }
                    egui::paint_texture_at(painter, rect, &options, &texture);
                }
            }
        }
    }
}

/// Draws a menu, or a caller's widget.
type AddUi<'a> = Box<dyn FnOnce(&mut Ui) + 'a>;

/// What a tool draws besides its button.
enum Body<'a> {
    None,
    /// A menu's content.
    Menu(AddUi<'a>),
    /// A caller's widget, of a size.
    Custom(Vec2, AddUi<'a>),
}

/// A tool of a [`ToolRow`]: made with one of the constructors, then given an icon, a tooltip,
/// and whether it is enabled or busy.
///
/// A tool's id names it in [`ToolRowResponse::clicked`] and [`laid_out`], and gives its widget
/// id: unique in its row, and stable from one pass to the next.
pub struct Tool<'a> {
    id: String,
    kind: ToolKind,
    label: String,
    icon: Option<ToolIcon<'a>>,
    tooltip: Option<String>,
    shortcut: Option<String>,
    enabled: bool,
    checked: bool,
    busy: bool,
    body: Body<'a>,
}

impl std::fmt::Debug for Tool<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tool")
            .field("id", &self.id)
            .field("kind", &self.kind)
            .field("label", &self.label)
            .field("icon", &self.icon)
            .field("enabled", &self.enabled)
            .field("checked", &self.checked)
            .field("busy", &self.busy)
            .finish_non_exhaustive()
    }
}

impl<'a> Tool<'a> {
    fn new(id: impl Into<String>, kind: ToolKind, label: impl Into<String>) -> Self {
        Tool {
            id: id.into(),
            kind,
            label: label.into(),
            icon: None,
            tooltip: None,
            shortcut: None,
            enabled: true,
            checked: false,
            busy: false,
            body: Body::None,
        }
    }

    /// A button: its click is [`ToolRowResponse::clicked`]. `label` names it, for screen
    /// readers too, and shows as the [`ToolDisplay`] say.
    pub fn button(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self::new(id, ToolKind::Button, label)
    }

    /// A button on or off, `checked`: framed while on, announced as toggled. Its click is
    /// [`ToolRowResponse::clicked`]; the caller turns it over.
    pub fn toggle(id: impl Into<String>, label: impl Into<String>, checked: bool) -> Self {
        let mut tool = Self::new(id, ToolKind::Toggle, label);
        tool.checked = checked;
        tool
    }

    /// A choice among others (an alignment, a tool of a palette), `selected` or not: drawn as
    /// a toggle, announced as a radio button.
    pub fn radio(id: impl Into<String>, label: impl Into<String>, selected: bool) -> Self {
        let mut tool = Self::new(id, ToolKind::Radio, label);
        tool.checked = selected;
        tool
    }

    /// A button with a drop-down mark, opening a menu that `add` draws (called only while
    /// open). In the More menu it is a submenu.
    pub fn menu(
        id: impl Into<String>,
        label: impl Into<String>,
        add: impl FnOnce(&mut Ui) + 'a,
    ) -> Self {
        let mut tool = Self::new(id, ToolKind::Menu, label);
        tool.body = Body::Menu(Box::new(add));
        tool
    }

    /// The line between two groups.
    pub fn separator() -> Self {
        Self::new("", ToolKind::Separator, "")
    }

    /// Room that pushes the tools after it to the end of the row, shared with the other
    /// spacers. It is the first room given up when the tools do not fit.
    pub fn spacer() -> Self {
        Self::new("", ToolKind::Spacer, "")
    }

    /// A widget of the caller's own (a field, a zoom box) in `size` points: `add` draws it in
    /// that room, or in the More menu when it does not fit, where `label` names nothing but the
    /// tool in [`laid_out`]. The row does not click it, name it or move the focus over it.
    pub fn custom(
        id: impl Into<String>,
        label: impl Into<String>,
        size: Vec2,
        add: impl FnOnce(&mut Ui) + 'a,
    ) -> Self {
        let mut tool = Self::new(id, ToolKind::Custom, label);
        tool.body = Body::Custom(size.max(Vec2::ZERO), Box::new(add));
        tool
    }

    /// Its icon: an [`Image`], or a [`ToolIcon::Paint`] closure.
    pub fn icon(mut self, icon: impl Into<ToolIcon<'a>>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    /// Its icon, painted by a closure in the square and the colour it is given.
    pub fn icon_painter(mut self, paint: impl Fn(&Painter, Rect, Color32) + 'a) -> Self {
        self.icon = Some(ToolIcon::Paint(Box::new(paint)));
        self
    }

    /// Its tooltip (a label with its key, `Save (Ctrl+S)`, or why it is disabled). By default
    /// its label, when the label does not show.
    pub fn tooltip(mut self, tooltip: impl Into<String>) -> Self {
        self.tooltip = Some(tooltip.into());
        self
    }

    /// Its key binding as text (`Ctrl+S`), shown at the end of its row in the More menu, and
    /// in its default tooltip (`Save (Ctrl+S)`) when its label does not show.
    pub fn shortcut(mut self, shortcut: impl Into<String>) -> Self {
        self.shortcut = Some(shortcut.into());
        self
    }

    /// Whether it takes clicks and the focus (yes by default); disabled, it is greyed and
    /// keeps its tooltip.
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Whether it waits for the result of what it started: a spinner in its icon's place,
    /// announced busy. It still clicks unless disabled.
    pub fn busy(mut self, busy: bool) -> Self {
        self.busy = busy;
        self
    }
}

/// A tool as drawn on the last pass ([`laid_out`]).
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct LaidTool {
    /// Its id, empty for a separator or a spacer.
    pub id: String,
    /// What it is.
    pub kind: ToolKind,
    /// Its label, its name for screen readers.
    pub label: String,
    /// Whether it has an icon.
    pub icon: bool,
    /// Whether it takes clicks.
    pub enabled: bool,
    /// A toggle on, a radio choice selected.
    pub checked: bool,
    /// Whether it waits, a spinner shown.
    pub busy: bool,
    /// It did not fit: it is in the More menu.
    pub overflow: bool,
    /// Where it is in the row; none in the More menu.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub rect: Option<Rect>,
    /// Its widget id in the row, for a button, a toggle, a radio choice or a menu: what
    /// [`focus`] and egui's memory go by.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub widget: Option<Id>,
}

/// A row as drawn on the last pass ([`laid_out`]).
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct LaidRow {
    /// Which way it runs.
    pub orientation: Orientation,
    /// What its tools show.
    pub display: ToolDisplay,
    /// Its title, its name for screen readers.
    pub title: Option<String>,
    /// Every tool, in order, those in the More menu marked
    /// [`overflow`](LaidTool::overflow). A separator or a spacer left out at the cut is not
    /// listed.
    pub tools: Vec<LaidTool>,
    /// How many tools are in the More menu, separators not counted.
    pub overflow: usize,
    /// The More button's widget id, when it shows.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub more: Option<Id>,
    /// The whole row.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub rect: Rect,
}

/// What the user did to a row this pass ([`ToolRow::show`]).
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct ToolRowResponse {
    /// The tool clicked, by id, in the row or in the More menu, with the pointer or with Enter
    /// or Space: a button, a toggle or a radio choice.
    pub clicked: Option<String>,
    /// The tool that has the keyboard focus, by id, if one of the row's has it (the More
    /// button's is [`MORE`]).
    pub focused: Option<String>,
    /// The row as drawn.
    pub laid: LaidRow,
    /// The whole row's response (hover only).
    pub response: Response,
}

/// The id [`focused`] and [`focus`] give the More button.
pub const MORE: &str = "\u{0}more";

/// The row named `name` as drawn on the last pass, if it was.
pub fn laid_out(ctx: &Context, name: &str) -> Option<LaidRow> {
    ctx.data(|d| d.get_temp::<LaidRow>(row_id(name).with("laid")))
}

/// The tool of the row named `name` that has the keyboard focus, by id ([`MORE`] for the More
/// button), as of the last pass.
pub fn focused(ctx: &Context, name: &str) -> Option<String> {
    let laid = laid_out(ctx, name)?;
    let focus = ctx.memory(|m| m.focused())?;
    if laid.more == Some(focus) {
        return Some(MORE.to_owned());
    }
    laid.tools
        .into_iter()
        .find(|t| t.widget == Some(focus))
        .map(|t| t.id)
}

/// Gives the keyboard focus to a tool of the row named `name`, by id ([`MORE`] for the More
/// button), and makes it the row's Tab stop. Says whether the row, on its last pass, showed
/// that tool enabled in the row (not in the More menu).
pub fn focus(ctx: &Context, name: &str, tool: &str) -> bool {
    let Some(laid) = laid_out(ctx, name) else {
        return false;
    };
    let widget = if tool == MORE {
        laid.more
    } else {
        laid.tools
            .iter()
            .find(|t| t.id == tool && t.enabled && !t.overflow)
            .and_then(|t| t.widget)
    };
    let Some(widget) = widget else {
        return false;
    };
    let id = row_id(name);
    ctx.data_mut(|d| d.insert_temp(id.with("stop"), widget));
    ctx.memory_mut(|m| m.request_focus(widget));
    true
}

fn row_id(name: &str) -> Id {
    Id::unique(("egui_stack.toolbar", name))
}

/// A row (or a column) of tools, named for its memory and for [`laid_out`]. See the
/// [module](self) for its behaviour.
pub struct ToolRow<'a> {
    name: String,
    title: Option<String>,
    orientation: Orientation,
    display: ToolDisplay,
    tab_focus: TabFocus,
    icon_size: f32,
    padding: f32,
    spacing: f32,
    painter: &'a (dyn DockPainter + Sync),
    texts: Labels,
}

impl std::fmt::Debug for ToolRow<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolRow")
            .field("name", &self.name)
            .field("title", &self.title)
            .field("orientation", &self.orientation)
            .field("display", &self.display)
            .field("tab_focus", &self.tab_focus)
            .finish_non_exhaustive()
    }
}

/// What a row's tools are sized from.
struct Metrics {
    icon: f32,
    pad: f32,
    gap: f32,
    /// A button's height in a row, its least width.
    height: f32,
    /// A separator's room along the row.
    separator: f32,
    /// A mark's side.
    mark: f32,
}

/// A tool worked out for this pass.
struct Placed {
    galley: Option<Arc<Galley>>,
    /// The key binding laid out, in the More menu.
    shortcut: Option<Arc<Galley>>,
    show_icon: bool,
    /// Its width in a row.
    width: f32,
    /// Its height in a row.
    height: f32,
    /// Its room along the row.
    extent: f32,
}

impl<'a> ToolRow<'a> {
    /// A row named `name`, unique in the application: [`laid_out`] reads it and its Tab stop is
    /// remembered under it. A row of icons, across, one Tab stop.
    pub fn new(name: impl Into<String>) -> Self {
        ToolRow {
            name: name.into(),
            title: None,
            orientation: Orientation::Horizontal,
            display: ToolDisplay::Icons,
            tab_focus: TabFocus::Roving,
            icon_size: 16.0,
            padding: 4.0,
            spacing: 2.0,
            painter: &StylePainter,
            texts: Labels::ENGLISH,
        }
    }

    /// Its name for screen readers ("Formatting"): its More button is then "More tools:
    /// Formatting" ([`Labels::more_tools_in`]). Give one whenever several rows show at a time.
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// Which way it runs. A column measures the height its `Ui` offers: inside
    /// `ui.horizontal` that is one row's, so give it room of its own.
    pub fn orientation(mut self, orientation: Orientation) -> Self {
        self.orientation = orientation;
        self
    }

    /// A column rather than a row (a tool palette).
    pub fn vertical(self) -> Self {
        self.orientation(Orientation::Vertical)
    }

    /// Icons alone (the default), icons with their labels, or labels alone.
    pub fn display(mut self, display: ToolDisplay) -> Self {
        self.display = display;
        self
    }

    /// One Tab stop for the row with the arrows along it ([`TabFocus::Roving`], the default),
    /// or every tool a Tab stop ([`TabFocus::Each`]), for an application that moves the focus
    /// along the row itself.
    pub fn tab_focus(mut self, tab_focus: TabFocus) -> Self {
        self.tab_focus = tab_focus;
        self
    }

    /// The side of an icon, in points (16 by default).
    pub fn icon_size(mut self, points: f32) -> Self {
        self.icon_size = points.max(1.0);
        self
    }

    /// The room around a tool's icon and label, in points (4 by default).
    pub fn padding(mut self, points: f32) -> Self {
        self.padding = points.max(0.0);
        self
    }

    /// The room between two tools, in points (2 by default).
    pub fn spacing(mut self, points: f32) -> Self {
        self.spacing = points.max(0.0);
        self
    }

    /// The painter of the tools, the More button and the separators, [`StylePainter`] by
    /// default ([`DockPainter::tool_item`], [`DockPainter::tool_more`],
    /// [`DockPainter::tool_separator`]).
    pub fn painter(mut self, painter: &'a (dyn DockPainter + Sync)) -> Self {
        self.painter = painter;
        self
    }

    /// The accessible names, English by default: the More button is named from
    /// [`Labels::more_tools`] and [`Labels::more_tools_in`].
    pub fn labels(mut self, labels: Labels) -> Self {
        self.texts = labels;
        self
    }

    fn metrics(&self, ui: &Ui) -> Metrics {
        let text = ui.text_style_height(&TextStyle::Button);
        let height = (self.icon_size.max(text) + 2.0 * self.padding).round();
        Metrics {
            icon: self.icon_size,
            pad: self.padding,
            gap: self.spacing,
            height,
            separator: ui.spacing().item_spacing.x.max(1.0),
            mark: (self.icon_size * 0.6).round(),
        }
    }

    /// Works out what a tool shows and its size.
    fn place(&self, ui: &Ui, m: &Metrics, tool: &Tool<'_>) -> Placed {
        let vertical = self.orientation == Orientation::Vertical;
        match (&tool.body, tool.kind) {
            (Body::Custom(size, _), _) => Placed {
                galley: None,
                shortcut: None,
                show_icon: false,
                width: size.x,
                height: size.y,
                extent: if vertical { size.y } else { size.x },
            },
            (_, ToolKind::Separator) => Placed {
                galley: None,
                shortcut: None,
                show_icon: false,
                width: m.separator,
                height: m.separator,
                extent: m.separator,
            },
            (_, ToolKind::Spacer) => Placed {
                galley: None,
                shortcut: None,
                show_icon: false,
                width: 0.0,
                height: 0.0,
                extent: 0.0,
            },
            _ => {
                let show_icon = tool.icon.is_some() && self.display != ToolDisplay::Text;
                let text = self.display != ToolDisplay::Icons || !show_icon;
                let galley = text.then(|| label_galley(ui, &tool.label));
                let mut width = 2.0 * m.pad;
                if show_icon {
                    width += m.icon;
                }
                if let Some(g) = &galley {
                    if show_icon {
                        width += m.pad;
                    }
                    width += g.size().x;
                }
                if tool.kind == ToolKind::Menu {
                    width += m.pad + m.mark;
                }
                let width = width.max(m.height).ceil();
                Placed {
                    galley,
                    shortcut: None,
                    show_icon,
                    width,
                    height: m.height,
                    extent: if vertical { m.height } else { width },
                }
            }
        }
    }

    /// Lays the tools out in the room left in `ui`, the ones that do not fit in the More menu.
    pub fn show<'t>(
        self,
        ui: &mut Ui,
        tools: impl IntoIterator<Item = Tool<'t>>,
    ) -> ToolRowResponse {
        let tools: Vec<Tool<'t>> = tools.into_iter().collect();
        let ctx = ui.ctx().clone();
        let id = row_id(&self.name);
        let m = self.metrics(ui);
        let vertical = self.orientation == Orientation::Vertical;
        let roving = self.tab_focus == TabFocus::Roving;
        let placed: Vec<Placed> = tools.iter().map(|t| self.place(ui, &m, t)).collect();

        // What fits.
        let available = ui.available_rect_before_wrap();
        let room = if vertical {
            available.height()
        } else {
            available.width()
        };
        let room = if room.is_finite() {
            room
        } else {
            f32::INFINITY
        };
        let extents: Vec<f32> = placed.iter().map(|p| p.extent).collect();
        let kinds: Vec<ToolKind> = tools.iter().map(|t| t.kind).collect();
        let cut = cut(&kinds, &extents, m.gap, m.height, room);
        let has_more = !cut.overflow.is_empty();

        // The row's size: across, its tallest tool (its widest, in a column).
        let cross = cut
            .shown
            .iter()
            .map(|&i| {
                let p = &placed[i];
                match (kinds[i], vertical) {
                    (ToolKind::Separator | ToolKind::Spacer, _) => 0.0,
                    (_, true) => p.width,
                    (_, false) => p.height,
                }
            })
            .fold(m.height, f32::max);
        let along: f32 = cut.shown.iter().map(|&i| extents[i]).sum::<f32>()
            + if has_more { m.height } else { 0.0 };
        let items = cut.shown.len() + usize::from(has_more);
        let along = along + m.gap * items.saturating_sub(1) as f32;
        // The spacers share what is left.
        let spacers = cut
            .shown
            .iter()
            .filter(|&&i| kinds[i] == ToolKind::Spacer)
            .count();
        let spare = if spacers > 0 && room.is_finite() {
            (room - along).max(0.0) / spacers as f32
        } else {
            0.0
        };
        let along = along + spare * spacers as f32;
        let size = if vertical {
            vec2(cross, along)
        } else {
            vec2(along, cross)
        };
        let rect = Rect::from_min_size(available.min, size);

        let mut builder = UiBuilder::new()
            .id_salt(("egui_stack.toolbar", &self.name))
            .max_rect(rect)
            .accessibility_role(Role::Toolbar);
        if let Some(title) = &self.title {
            builder = builder.accessibility_label(title.clone());
        }
        let mut row = ui.new_child(builder);
        ctx.accesskit_node_builder(row.unique_id(), |node| {
            node.set_orientation(if vertical {
                accesskit::Orientation::Vertical
            } else {
                accesskit::Orientation::Horizontal
            });
        });

        // The Tab stop: the tool focused there last, else the first that takes the focus.
        let widget = |tool: &Tool<'_>| id.with(("tool", &tool.id));
        let more_id = id.with("more");
        let mut group: Vec<Id> = cut
            .shown
            .iter()
            .filter(|&&i| tools[i].kind.is_button() && tools[i].enabled)
            .map(|&i| widget(&tools[i]))
            .collect();
        if has_more {
            group.push(more_id);
        }
        let remembered: Option<Id> = ctx.data(|d| d.get_temp(id.with("stop")));
        let stop = remembered
            .filter(|r| group.contains(r))
            .or_else(|| group.first().copied());
        let sense = |widget: Id| {
            if roving && Some(widget) != stop {
                Sense::CLICK
            } else {
                Sense::click()
            }
        };

        let mut clicked = None;
        let mut popup_open = false;
        let mut laid_tools: Vec<LaidTool> = Vec::with_capacity(tools.len());
        let mut at = rect.min;
        let step = |at: &mut Pos2, extent: f32| {
            if vertical {
                at.y += extent + m.gap;
            } else {
                at.x += extent + m.gap;
            }
        };
        let mut overflowed: Vec<(Tool<'t>, Placed)> = Vec::new();
        for (i, (tool, p)) in tools.into_iter().zip(placed).enumerate() {
            let shown = cut.shown.contains(&i);
            if !shown {
                if cut.overflow.contains(&i) {
                    laid_tools.push(laid(&tool, true, None, None));
                    overflowed.push((tool, p));
                }
                continue;
            }
            let extent = if tool.kind == ToolKind::Spacer {
                spare
            } else {
                p.extent
            };
            let slot = if vertical {
                Rect::from_min_size(at, vec2(cross, extent))
            } else {
                Rect::from_min_size(at, vec2(extent, cross))
            };
            step(&mut at, extent);
            match tool.kind {
                ToolKind::Separator => {
                    if row.is_rect_visible(slot) {
                        self.painter.tool_separator(
                            row.painter(),
                            &ToolSeparatorPaint {
                                rect: slot,
                                vertical,
                                style: row.style(),
                            },
                        );
                    }
                    laid_tools.push(laid(&tool, false, Some(slot), None));
                }
                ToolKind::Spacer => laid_tools.push(laid(&tool, false, Some(slot), None)),
                ToolKind::Custom => {
                    let tool_rect = Rect::from_center_size(slot.center(), vec2(p.width, p.height));
                    let tool_rect = if vertical {
                        tool_rect.translate(vec2(slot.left() - tool_rect.left(), 0.0))
                    } else {
                        tool_rect
                    };
                    laid_tools.push(laid(&tool, false, Some(tool_rect), None));
                    if let Body::Custom(_, add) = tool.body {
                        let mut child = row.new_child(
                            UiBuilder::new()
                                .id_salt(("custom", &tool.id))
                                .max_rect(tool_rect),
                        );
                        add(&mut child);
                    }
                }
                _ => {
                    let w = widget(&tool);
                    let tool_rect = slot;
                    let r = if tool.enabled {
                        row.interact(tool_rect, w, sense(w))
                    } else {
                        row.interact(tool_rect, w, Sense::hover())
                    };
                    let open = tool.kind == ToolKind::Menu
                        && Popup::is_id_open(&ctx, Popup::default_response_id(&r));
                    popup_open |= open;
                    let state = part_state(&r)
                        .with(PartState::SELECTED, tool.checked || open)
                        .with(PartState::DISABLED, !tool.enabled);
                    self.paint_tool(&row, &m, &tool, &p, tool_rect, r.id, state, false);
                    name_tool(&ctx, &r, &tool, open);
                    let r = tooltip(r, &tool, p.galley.is_some());
                    laid_tools.push(laid(&tool, false, Some(tool_rect), Some(w)));
                    if r.clicked() {
                        ctx.data_mut(|d| d.insert_temp(id.with("stop"), w));
                    }
                    match tool.body {
                        Body::Menu(add) => {
                            menu_popup(&r).show(add);
                        }
                        _ => {
                            if r.clicked() {
                                clicked = Some(tool.id.clone());
                            }
                        }
                    }
                }
            }
        }

        // The More button and its menu.
        let mut more = None;
        if has_more {
            let slot = Rect::from_min_size(at, Vec2::splat(m.height));
            let slot = if vertical {
                Rect::from_min_size(at, vec2(cross, m.height))
            } else {
                slot.translate(vec2(0.0, (cross - m.height) / 2.0))
            };
            let r = row.interact(slot, more_id, sense(more_id));
            let open = Popup::is_id_open(&ctx, Popup::default_response_id(&r));
            popup_open |= open;
            if row.is_rect_visible(slot) {
                self.painter.tool_more(
                    row.painter(),
                    &ToolMorePaint {
                        rect: slot,
                        id: more_id,
                        vertical,
                        state: part_state(&r).with(PartState::SELECTED, open),
                        style: row.style(),
                    },
                );
            }
            let name = match &self.title {
                Some(title) => self.texts.more_tools_in.replace("{toolbar}", title),
                None => self.texts.more_tools.to_string(),
            };
            {
                let name = name.clone();
                r.widget_info(|| WidgetInfo::labeled(Role::Button, true, &name));
            }
            ctx.accesskit_node_builder(r.id, |node| {
                node.set_expanded(open);
                node.set_has_popup(accesskit::HasPopup::Menu);
            });
            let r = r.on_hover_text(name);
            if r.clicked() {
                ctx.data_mut(|d| d.insert_temp(id.with("stop"), more_id));
            }
            menu_popup(&r).show(|ui| {
                if let Some(id) = self.more_menu(ui, &m, id, overflowed) {
                    clicked = Some(id);
                }
            });
            more = Some(more_id);
        }

        // Assistive technology may focus any tool, not only the Tab stop: it becomes the stop.
        let asked = group
            .iter()
            .copied()
            .find(|&g| ctx.input(|i| i.has_accesskit_action_request(g, accesskit::Action::Focus)));
        if let Some(to) = asked {
            ctx.data_mut(|d| d.insert_temp(id.with("stop"), to));
            ctx.memory_mut(|mem| mem.request_focus(to));
        }

        // The arrows, Home and End along the row, unless a menu of it is open.
        let focus = ctx.memory(|m| m.focused());
        let focused_at = focus.and_then(|f| group.iter().position(|&g| g == f));
        if let Some(at) = focused_at {
            ctx.data_mut(|d| d.insert_temp(id.with("stop"), group[at]));
            if roving && !popup_open {
                let (back, forth) = if vertical {
                    (Key::ArrowUp, Key::ArrowDown)
                } else {
                    (Key::ArrowLeft, Key::ArrowRight)
                };
                ctx.memory_mut(|mem| {
                    mem.set_focus_lock_filter(
                        group[at],
                        EventFilter {
                            horizontal_arrows: !vertical,
                            vertical_arrows: vertical,
                            ..Default::default()
                        },
                    );
                });
                let n = group.len();
                let target = ctx.input(|i| {
                    if i.key_pressed(forth) {
                        Some((at + 1) % n)
                    } else if i.key_pressed(back) {
                        Some((at + n - 1) % n)
                    } else if i.key_pressed(Key::Home) {
                        Some(0)
                    } else if i.key_pressed(Key::End) {
                        Some(n - 1)
                    } else {
                        None
                    }
                });
                if let Some(target) = target.filter(|&t| t != at) {
                    let to = group[target];
                    ctx.data_mut(|d| d.insert_temp(id.with("stop"), to));
                    ctx.memory_mut(|mem| mem.request_focus(to));
                }
            }
        }
        let focused = focus.and_then(|f| {
            if more == Some(f) {
                Some(MORE.to_owned())
            } else {
                laid_tools
                    .iter()
                    .find(|t| t.widget == Some(f))
                    .map(|t| t.id.clone())
            }
        });

        let response = ui.allocate_rect(rect, Sense::hover());
        let laid = LaidRow {
            orientation: self.orientation,
            display: self.display,
            title: self.title.clone(),
            overflow: laid_tools
                .iter()
                .filter(|t| t.overflow && t.kind != ToolKind::Separator)
                .count(),
            tools: laid_tools,
            more,
            rect,
        };
        ctx.data_mut(|d| d.insert_temp(id.with("laid"), laid.clone()));
        ToolRowResponse {
            clicked,
            focused,
            laid,
            response,
        }
    }

    /// Paints a button of the row, or a row of the More menu.
    #[allow(clippy::too_many_arguments)]
    fn paint_tool(
        &self,
        ui: &Ui,
        m: &Metrics,
        tool: &Tool<'_>,
        p: &Placed,
        rect: Rect,
        id: Id,
        state: PartState,
        in_menu: bool,
    ) {
        if !ui.is_rect_visible(rect) {
            return;
        }
        let y = rect.center().y;
        let square = |x: f32| Rect::from_min_size(pos2(x, y - m.icon / 2.0), Vec2::splat(m.icon));
        // Alone, the icon is centred; with a label, both start at the padding. A column's
        // tools all start there.
        let alone = !in_menu && p.galley.is_none() && tool.kind != ToolKind::Menu;
        let (icon_rect, mut x) = if alone {
            let at = Rect::from_center_size(rect.center(), Vec2::splat(m.icon));
            (at, at.right())
        } else if p.show_icon || in_menu {
            let at = square(rect.left() + m.pad);
            (at, at.right() + m.pad)
        } else {
            (square(rect.left() + m.pad), rect.left() + m.pad)
        };
        let label_pos = match &p.galley {
            Some(g) => {
                let pos = pos2(x, y - g.size().y / 2.0);
                x += g.size().x;
                pos
            }
            None => pos2(x, y),
        };
        let marked = tool.kind == ToolKind::Menu
            || (in_menu && matches!(tool.kind, ToolKind::Toggle | ToolKind::Radio));
        let mark = marked.then(|| {
            let right = if in_menu {
                rect.right() - m.pad - m.mark
            } else {
                (x + m.pad).max(rect.right() - m.pad - m.mark)
            };
            Rect::from_min_size(pos2(right, y - m.mark / 2.0), Vec2::splat(m.mark))
        });
        let show_icon = p.show_icon || (in_menu && tool.icon.is_some());
        // A key binding ends before the mark's room.
        let shortcut_pos = p.shortcut.as_ref().map_or(pos2(x, y), |g| {
            let end = rect.right() - 2.0 * m.pad - m.mark;
            pos2(end - g.size().x, y - g.size().y / 2.0)
        });
        self.painter.tool_item(
            ui.painter(),
            &ToolItemPaint {
                rect,
                id,
                kind: tool.kind,
                icon: tool.icon.as_ref().filter(|_| show_icon),
                icon_rect,
                label: p.galley.as_ref(),
                label_pos,
                shortcut: p.shortcut.as_ref(),
                shortcut_pos,
                mark,
                busy: tool.busy,
                in_menu,
                state,
                style: ui.style(),
            },
        );
    }

    /// Draws the tools that did not fit as a menu's rows; says which was clicked.
    fn more_menu(
        &self,
        ui: &mut Ui,
        m: &Metrics,
        row: Id,
        tools: Vec<(Tool<'_>, Placed)>,
    ) -> Option<String> {
        // Every row shows its icon's room and its label, so that the labels line up.
        let tools: Vec<(Tool<'_>, Placed)> = tools
            .into_iter()
            .map(|(tool, mut p)| {
                if tool.kind.is_button() {
                    p.galley = Some(label_galley(ui, &tool.label));
                    p.shortcut = tool.shortcut.as_deref().map(|k| label_galley(ui, k));
                    p.show_icon = tool.icon.is_some();
                }
                (tool, p)
            })
            .collect();
        let width = tools
            .iter()
            .filter(|(t, _)| t.kind.is_button())
            .map(|(_, p)| {
                let label = p.galley.as_ref().map_or(0.0, |g| g.size().x);
                let keys = p
                    .shortcut
                    .as_ref()
                    .map_or(0.0, |g| 2.0 * m.pad + g.size().x);
                4.0 * m.pad + m.icon + label + keys + m.mark
            })
            .fold(0.0, f32::max);
        ui.set_min_width(width);
        let mut clicked = None;
        for (tool, p) in tools {
            match tool.kind {
                ToolKind::Separator => {
                    ui.separator();
                }
                ToolKind::Spacer => {}
                ToolKind::Custom => {
                    if let Body::Custom(_, add) = tool.body {
                        add(ui);
                    }
                }
                ToolKind::Menu => {
                    if let Body::Menu(add) = tool.body {
                        ui.add_enabled_ui(tool.enabled, |ui| {
                            ui.menu_button(tool.label.as_str(), add);
                        });
                    }
                }
                _ => {
                    let w = row.with(("more", &tool.id));
                    let size = vec2(width, m.height);
                    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
                    let sense = if tool.enabled {
                        Sense::click()
                    } else {
                        Sense::hover()
                    };
                    let r = ui.interact(rect, w, sense);
                    let state = part_state(&r)
                        .with(PartState::SELECTED, tool.checked)
                        .with(PartState::DISABLED, !tool.enabled);
                    self.paint_tool(ui, m, &tool, &p, rect, w, state, true);
                    name_tool(ui.ctx(), &r, &tool, false);
                    let r = match &tool.tooltip {
                        Some(tip) => r.on_hover_text(tip),
                        None => r,
                    };
                    if r.clicked() {
                        clicked = Some(tool.id.clone());
                        ui.close();
                    }
                }
            }
        }
        clicked
    }
}

/// A tool's label laid out, to be coloured by the painter.
fn label_galley(ui: &Ui, label: &str) -> Arc<Galley> {
    let font = TextStyle::Button.resolve(ui.style());
    ui.painter()
        .layout_no_wrap(label.to_owned(), font, Color32::PLACEHOLDER)
}

/// What a tool's click opens: a menu under it.
fn menu_popup(r: &Response) -> Popup<'_> {
    let config = MenuConfig::default();
    Popup::menu(r)
        .close_behavior(config.close_behavior)
        .info(UiStackInfo::new(UiKind::Menu).with_tag_value(MenuConfig::MENU_CONFIG_TAG, config))
}

/// The tooltip a tool shows: its own, or its label when the label does not show.
fn tooltip(r: Response, tool: &Tool<'_>, label_shown: bool) -> Response {
    let tip = match (&tool.tooltip, &tool.shortcut) {
        (Some(tip), _) => tip.clone(),
        (None, _) if label_shown => return r,
        (None, Some(keys)) => format!("{} ({keys})", tool.label),
        (None, None) => tool.label.clone(),
    };
    r.on_hover_text(tip)
}

/// Names a tool for assistive technology: a button, a toggle, a radio button or a menu button.
fn name_tool(ctx: &Context, r: &Response, tool: &Tool<'_>, open: bool) {
    let (enabled, checked) = (tool.enabled, tool.checked);
    let name = tool.label.as_str();
    match tool.kind {
        ToolKind::Toggle => {
            r.widget_info(|| WidgetInfo::selected(Role::Button, enabled, checked, name));
        }
        ToolKind::Radio => {
            r.widget_info(|| WidgetInfo::selected(Role::RadioButton, enabled, checked, name));
        }
        _ => r.widget_info(|| WidgetInfo::labeled(Role::Button, enabled, name)),
    }
    ctx.accesskit_node_builder(r.id, |node| {
        if tool.kind == ToolKind::Menu {
            node.set_expanded(open);
            node.set_has_popup(accesskit::HasPopup::Menu);
        }
        if tool.busy {
            node.set_busy();
        }
    });
}

fn laid(tool: &Tool<'_>, overflow: bool, rect: Option<Rect>, widget: Option<Id>) -> LaidTool {
    LaidTool {
        id: tool.id.clone(),
        kind: tool.kind,
        label: tool.label.clone(),
        icon: tool.icon.is_some(),
        enabled: tool.enabled,
        checked: tool.checked,
        busy: tool.busy,
        overflow,
        rect,
        widget,
    }
}

/// A part's state for the painter.
fn part_state(r: &Response) -> PartState {
    PartState::NONE
        .with(PartState::HOVERED, r.hovered())
        .with(PartState::PRESSED, r.is_pointer_button_down_on())
        .with(PartState::FOCUSED, r.has_focus())
}

/// Which tools a row shows and which go into the More menu, by index, in order.
#[derive(Debug, PartialEq)]
struct Cut {
    shown: Vec<usize>,
    overflow: Vec<usize>,
}

/// Cuts a row of tools of these kinds and extents (along the row) in `room` points, `gap`
/// apart, the More button `more` long: all of them when they fit. A spacer takes no room here;
/// a separator or a spacer never ends the row once cut, a separator never starts nor ends the
/// More menu, two never follow each other there, and the menu holds no spacer.
fn cut(kinds: &[ToolKind], extents: &[f32], gap: f32, more: f32, room: f32) -> Cut {
    let all: f32 = extents.iter().sum::<f32>() + gap * extents.len().saturating_sub(1) as f32;
    let kept = if all <= room + 0.01 {
        extents.len()
    } else {
        let mut used = more;
        extents
            .iter()
            .take_while(|e| {
                used += *e + gap;
                used <= room + 0.01
            })
            .count()
    };
    let gap_kind = |k: ToolKind| matches!(k, ToolKind::Separator | ToolKind::Spacer);
    let mut end = kept;
    if kept < extents.len() {
        while end > 0 && gap_kind(kinds[end - 1]) {
            end -= 1;
        }
    }
    let shown: Vec<usize> = (0..end).collect();
    let mut overflow: Vec<usize> = Vec::new();
    for i in kept..extents.len() {
        match kinds[i] {
            ToolKind::Spacer => {}
            ToolKind::Separator => {
                let after_tool = overflow
                    .last()
                    .is_some_and(|&j| kinds[j] != ToolKind::Separator);
                if after_tool {
                    overflow.push(i);
                }
            }
            _ => overflow.push(i),
        }
    }
    while overflow
        .last()
        .is_some_and(|&j| kinds[j] == ToolKind::Separator)
    {
        overflow.pop();
    }
    Cut { shown, overflow }
}

#[cfg(test)]
mod tests {
    use super::{Cut, ToolKind, cut};

    const B: ToolKind = ToolKind::Button;
    const S: ToolKind = ToolKind::Separator;
    const P: ToolKind = ToolKind::Spacer;

    fn c(shown: &[usize], overflow: &[usize]) -> Cut {
        Cut {
            shown: shown.to_vec(),
            overflow: overflow.to_vec(),
        }
    }

    #[test]
    fn tools_that_do_not_fit_go_into_more_from_the_end() {
        let k = [B, B, B];
        let e = [10.0, 10.0, 10.0];
        // All fit: no More button.
        assert_eq!(cut(&k, &e, 1.0, 10.0, 32.0), c(&[0, 1, 2], &[]));
        // One goes: the More button takes its room too, and a gap before it.
        assert_eq!(cut(&k, &e, 1.0, 8.0, 31.0), c(&[0, 1], &[2]));
        assert_eq!(cut(&k, &e, 1.0, 10.0, 31.0), c(&[0], &[1, 2]));
        // Nothing fits besides it.
        assert_eq!(cut(&k, &e, 1.0, 10.0, 5.0), c(&[], &[0, 1, 2]));
        // Endless room.
        assert_eq!(cut(&k, &e, 1.0, 10.0, f32::INFINITY), c(&[0, 1, 2], &[]));
    }

    #[test]
    fn a_separator_never_ends_the_row_nor_starts_or_ends_the_menu() {
        let k = [B, S, B, S, S, B, S];
        let e = [10.0, 4.0, 10.0, 4.0, 4.0, 10.0, 4.0];
        // Cut after the first separator: it stays out of both.
        assert_eq!(cut(&k, &e, 0.0, 10.0, 25.0), c(&[0], &[2, 3, 5]));
    }

    #[test]
    fn a_spacer_takes_no_room_and_never_goes_into_the_menu() {
        let k = [B, P, B, B];
        let e = [10.0, 0.0, 10.0, 10.0];
        assert_eq!(cut(&k, &e, 0.0, 10.0, 30.0), c(&[0, 1, 2, 3], &[]));
        assert_eq!(cut(&k, &e, 0.0, 10.0, 21.0), c(&[0], &[2, 3]));
    }
}
