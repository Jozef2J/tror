use eframe::egui::{self, Color32, CornerRadius, Margin, Stroke};
use egui_stack::{DockState, NodeIndex, NodePath, Style, SurfaceIndex};
use serde::{Deserialize, Serialize};

use crate::theme::{self, Palette};

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub enum Pane {
    Welcome,
    Doc(u64),
    Hosts,
    Hosts2,
    Console,
    Log,
    ServerSearch,
    Settings,
    Diff,
    Db,
    Preview,
    Project,
    HostsN(u8),
    Plugin(u64),
    Project2,
}

static PLUGIN_PANES: std::sync::Mutex<Vec<(u64, String, String)>> = std::sync::Mutex::new(Vec::new());

pub fn set_plugin_panes(panes: Vec<(u64, String, String)>) {
    if let Ok(mut list) = PLUGIN_PANES.lock() {
        *list = panes;
    }
}

fn plugin_pane(key: u64) -> Option<(String, String)> {
    PLUGIN_PANES
        .lock()
        .ok()?
        .iter()
        .find(|p| p.0 == key)
        .map(|p| (p.1.clone(), p.2.clone()))
}

pub fn plugin_key(plugin: &str, panel: &str) -> u64 {
    format!("{plugin}/{panel}")
        .bytes()
        .fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ b as u64).wrapping_mul(0x0100_0000_01b3))
}

#[derive(PartialEq)]
enum Zone {
    Center,
    Left,
    Bottom,
}

impl Pane {
    pub fn hosts(index: usize) -> Pane {
        match index {
            0 => Pane::Hosts,
            1 => Pane::Hosts2,
            n => Pane::HostsN(n.min(250) as u8),
        }
    }

    pub fn host_index(self) -> Option<usize> {
        match self {
            Pane::Hosts => Some(0),
            Pane::Hosts2 => Some(1),
            Pane::HostsN(n) => Some(n as usize),
            _ => None,
        }
    }

    pub fn min_size(self) -> (f32, f32) {
        match self {
            Pane::Welcome | Pane::Doc(_) => (120.0, 60.0),
            Pane::Hosts | Pane::Hosts2 | Pane::HostsN(_) | Pane::Project | Pane::Project2 => (170.0, 120.0),
            Pane::Log => (200.0, 80.0),
            Pane::Console => (240.0, 100.0),
            Pane::ServerSearch => (240.0, 100.0),
            Pane::Db => (300.0, 160.0),
            Pane::Settings | Pane::Diff => (320.0, 160.0),
            Pane::Preview => (200.0, 120.0),
            Pane::Plugin(_) => (180.0, 100.0),
        }
    }

    pub fn title(self) -> String {
        match self {
            Pane::Hosts2 => "Подключение 2".into(),
            Pane::HostsN(n) => format!("Подключение {}", n as usize + 1),
            Pane::Plugin(key) => plugin_pane(key).map_or_else(|| "Плагин".into(), |p| p.0),
            other => other.label().into(),
        }
    }

    fn label(self) -> &'static str {
        match self {
            Pane::Welcome => "TrorPad",
            Pane::Doc(_) => "",
            Pane::Hosts => "Удалённые хосты",
            Pane::Hosts2 | Pane::HostsN(_) | Pane::Plugin(_) => "",
            Pane::Project => "Проект",
            Pane::Project2 => "Проект 2",
            Pane::Console => "Консоль",
            Pane::Log => "Журнал",
            Pane::ServerSearch => "Поиск на сервере",
            Pane::Settings => "Настройки",
            Pane::Diff => "Сравнение",
            Pane::Db => "База данных",
            Pane::Preview => "Предпросмотр",
        }
    }

    fn zone(self) -> Zone {
        match self {
            Pane::Hosts | Pane::Hosts2 | Pane::HostsN(_) | Pane::Project | Pane::Project2 => Zone::Left,
            Pane::Console | Pane::Log | Pane::ServerSearch => Zone::Bottom,
            Pane::Plugin(key) => match plugin_pane(key).map(|p| p.1).as_deref() {
                Some("bottom") => Zone::Bottom,
                Some("center") => Zone::Center,
                _ => Zone::Left,
            },
            _ => Zone::Center,
        }
    }
}

pub fn new() -> DockState<Pane> {
    let mut dock = DockState::new(vec![Pane::Welcome]);
    translate(&mut dock);
    dock
}

pub fn translate(dock: &mut DockState<Pane>) {
    let t = &mut dock.translations;
    t.tab_context_menu.close_button = "Закрыть".into();
    t.tab_context_menu.eject_button = "Сделать плавающей панелью".into();
    t.tab_context_menu.hide_tab_bar_button = "Скрыть заголовок".into();
    t.tab_context_menu.show_tab_bar_button = "Показать заголовок".into();
    t.leaf.close_button_disabled_tooltip = "Здесь есть вкладки, которые нельзя закрыть".into();
    t.leaf.close_all_button = "Закрыть окно".into();
    t.leaf.close_all_button_disabled_tooltip = "В окне есть вкладки, которые нельзя закрыть".into();
    t.leaf.minimize_button = "Свернуть окно".into();
}

pub fn has(dock: &DockState<Pane>, pane: Pane) -> bool {
    dock.find_tab(&pane).is_some()
}

pub fn hide(dock: &mut DockState<Pane>, pane: Pane) {
    if let Some(path) = dock.find_tab(&pane) {
        dock.remove_tab(path);
    }
}

fn zone_leaf(dock: &DockState<Pane>, zone: Zone) -> Option<NodePath> {
    let fits = |path: NodePath| {
        dock.leaf(path)
            .is_ok_and(|leaf| leaf.tabs().iter().any(|tab| tab.zone() == zone))
    };
    if let Some(path) = dock.focused_leaf()
        && fits(path)
    {
        return Some(path);
    }
    let main = dock
        .iter_leaves()
        .map(|(path, _)| path)
        .find(|path| path.surface == SurfaceIndex::main() && fits(*path));
    main.or_else(|| dock.iter_leaves().map(|(path, _)| path).find(|p| fits(*p)))
}

pub type Floats = Vec<(Pane, [f32; 4])>;

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Home {
    pub anchor: Pane,
    pub how: u8,
    pub depth: u8,
    pub share: f32,
}

thread_local! {
    static HOMES: std::cell::RefCell<Vec<(Pane, Home)>> = const { std::cell::RefCell::new(Vec::new()) };
}

pub fn homes() -> Vec<(Pane, Home)> {
    HOMES.with_borrow(|h| h.clone())
}

pub fn set_homes(homes: Vec<(Pane, Home)>) {
    HOMES.with_borrow_mut(|list| *list = homes);
}

fn first_tab(tree: &egui_stack::Tree<Pane>, node: NodeIndex) -> Option<Pane> {
    match &tree[node] {
        egui_stack::Node::Leaf(leaf) => leaf.tabs().first().copied(),
        other => other.children().iter().find_map(|c| first_tab(tree, *c)),
    }
}

fn side(tree: &egui_stack::Tree<Pane>, node: NodeIndex) -> bool {
    match &tree[node] {
        egui_stack::Node::Leaf(leaf) => !leaf.tabs().is_empty() && leaf.tabs().iter().all(steady),
        egui_stack::Node::Empty => false,
        other => other.children().iter().all(|c| side(tree, *c)),
    }
}

fn steady(tab: &Pane) -> bool {
    !matches!(tab, Pane::Doc(_) | Pane::Welcome)
}

pub fn remember_homes(dock: &DockState<Pane>) {
    let tree = dock.main_surface();
    let mut found = Vec::new();
    for (path, tab) in dock.iter_all_tabs() {
        if path.surface != SurfaceIndex::main() || !steady(tab) {
            continue;
        }
        let Some(leaf) = tree[path.node].get_leaf() else {
            continue;
        };
        if leaf.tabs().len() > 1 {
            let others = leaf.tabs().iter().filter(|t| *t != tab);
            let anchor = others.clone().find(|t| steady(t)).or_else(|| others.clone().next());
            if let Some(anchor) = anchor {
                found.push((*tab, Home { anchor: *anchor, how: 0, depth: 0, share: 0.5 }));
            }
            continue;
        }
        let Some(parent) = tree.parent(path.node) else {
            continue;
        };
        let Some(linear) = tree[parent].get_linear() else {
            continue;
        };
        let Some(index) = linear.position(path.node) else {
            continue;
        };
        let (other, before) = if index + 1 < linear.len() {
            (index + 1, true)
        } else if index > 0 {
            (index - 1, false)
        } else {
            continue;
        };
        let neighbour = linear.children()[other];
        let Some(anchor) = first_tab(tree, neighbour) else {
            continue;
        };
        let Some(anchor_path) = dock.find_tab(&anchor) else {
            continue;
        };
        let mut depth = 0u8;
        let mut node = anchor_path.node;
        while node != neighbour {
            match tree.parent(node) {
                Some(up) => {
                    node = up;
                    depth += 1;
                }
                None => break,
            }
        }
        let horizontal = matches!(linear.dir, egui_stack::LinearDir::Horizontal);
        let how = match (horizontal, before) {
            (true, true) => 1,
            (true, false) => 2,
            (false, true) => 3,
            (false, false) => 4,
        };
        let shares = linear.shares();
        let share = shares[index] / (shares[index] + shares[other]).max(f32::EPSILON);
        found.push((*tab, Home { anchor, how, depth, share }));
    }
    if found.is_empty() {
        return;
    }
    HOMES.with_borrow_mut(|list| {
        for (pane, home) in found {
            list.retain(|(p, _)| *p != pane);
            list.push((pane, home));
        }
    });
}

fn go_home(dock: &mut DockState<Pane>, pane: Pane) -> bool {
    let Some(home) = homes().into_iter().find(|(p, _)| *p == pane).map(|(_, h)| h) else {
        return false;
    };
    let Some(path) = dock.find_tab(&home.anchor) else {
        return false;
    };
    if path.surface != SurfaceIndex::main() {
        return false;
    }
    let tree = dock.main_surface_mut();
    if home.how == 0 {
        if let Some(leaf) = tree[path.node].get_leaf_mut() {
            leaf.append_tab(pane);
            return true;
        }
        return false;
    }
    let mut node = path.node;
    for _ in 0..home.depth {
        match tree.parent(node) {
            Some(up) => node = up,
            None => break,
        }
    }
    let keep = (1.0 - home.share).clamp(0.05, 0.95);
    match home.how {
        1 => tree.split_left(node, keep, vec![pane]),
        2 => tree.split_right(node, keep, vec![pane]),
        3 => tree.split_above(node, keep, vec![pane]),
        _ => tree.split_below(node, keep, vec![pane]),
    };
    true
}

#[derive(Default)]
pub struct Sizes {
    known: Vec<(Pane, f32)>,
    area: egui::Vec2,
}

pub fn keep_sizes(dock: &mut DockState<Pane>, sizes: &mut Sizes, area: egui::Vec2, dragging: bool) {
    let resized = (area - sizes.area).length() > 0.5;
    sizes.area = area;
    let tree = dock.main_surface_mut();
    let linears: Vec<NodeIndex> = (0..tree.len())
        .map(NodeIndex)
        .filter(|i| tree[*i].get_linear().is_some())
        .collect();
    for index in linears {
        let Some(linear) = tree[index].get_linear() else {
            continue;
        };
        let horizontal = matches!(linear.dir, egui_stack::LinearDir::Horizontal);
        let rect = linear.rect();
        let span = if horizontal { rect.width() } else { rect.height() };
        if !span.is_finite() || span < 50.0 {
            continue;
        }
        let children = linear.children().to_vec();
        let shares = linear.shares().to_vec();
        let total: f32 = shares.iter().sum();
        let sides: Vec<(usize, Pane)> = children
            .iter()
            .enumerate()
            .filter(|(_, c)| side(tree, **c))
            .filter_map(|(i, c)| Some((i, first_tab(tree, *c)?)))
            .collect();
        if sides.is_empty() || sides.len() == children.len() {
            continue;
        }
        if !resized {
            for (i, pane) in &sides {
                let known = sizes.known.iter().any(|(p, _)| p == pane);
                if dragging || !known {
                    sizes.known.retain(|(p, _)| p != pane);
                    sizes.known.push((*pane, shares[*i] / total * span));
                }
            }
            continue;
        }
        let mut updated = shares.clone();
        let mut fixed = 0.0;
        for (i, pane) in &sides {
            if let Some((_, px)) = sizes.known.iter().find(|(p, _)| p == pane) {
                updated[*i] = px / span * total;
            }
            fixed += updated[*i];
        }
        if fixed > total * 0.8 {
            let scale = total * 0.8 / fixed;
            for (i, _) in &sides {
                updated[*i] *= scale;
            }
            fixed = total * 0.8;
        }
        let flex: f32 = (0..children.len())
            .filter(|i| !sides.iter().any(|(s, _)| s == i))
            .map(|i| shares[i])
            .sum();
        for i in 0..children.len() {
            if !sides.iter().any(|(s, _)| *s == i) {
                updated[i] = if flex > 0.0 { shares[i] / flex * (total - fixed) } else { total - fixed };
            }
        }
        if let Some(linear) = tree[index].get_linear_mut() {
            linear.set_shares(updated);
        }
    }
}

pub fn show(dock: &mut DockState<Pane>, pane: Pane, floats: &Floats) {
    if let Some(path) = dock.find_tab(&pane) {
        let _ = dock.focus_tab(path);
        return;
    }
    let _ = floats;
    if steady(&pane) && go_home(dock, pane) {
        if let Some(path) = dock.find_tab(&pane) {
            let _ = dock.focus_tab(path);
        }
        return;
    }
    let twin = match pane {
        Pane::Hosts | Pane::Hosts2 | Pane::HostsN(_) => dock
            .iter_all_tabs()
            .map(|(_, tab)| *tab)
            .find(|tab| tab.host_index().is_some() && *tab != pane)
            .and_then(|tab| dock.find_tab(&tab)),
        _ => None,
    };
    let beside = (pane == Pane::Preview)
        .then(|| zone_leaf(dock, Zone::Center))
        .flatten()
        .filter(|path| path.surface == SurfaceIndex::main());
    if let Some(center) = beside {
        dock.main_surface_mut()
            .split_right(center.node, 0.5, vec![pane]);
    } else if let Some(twin) = twin.filter(|path| path.surface == SurfaceIndex::main()) {
        dock.main_surface_mut()
            .split_below(twin.node, 0.5, vec![pane]);
    } else if let Some(path) = zone_leaf(dock, pane.zone()) {
        if let Ok(leaf) = dock.leaf_mut(path) {
            leaf.append_tab(pane);
        }
    } else if dock.main_surface().is_empty() {
        dock.main_surface_mut().push_to_first_leaf(pane);
    } else {
        let center = zone_leaf(dock, Zone::Center)
            .filter(|path| path.surface == SurfaceIndex::main())
            .map(|path| path.node);
        let main = dock.main_surface_mut();
        match pane.zone() {
            Zone::Left => {
                main.split_left(NodeIndex::root(), 0.25, vec![pane]);
            }
            Zone::Bottom => {
                main.split_below(center.unwrap_or(NodeIndex::root()), 0.66, vec![pane]);
            }
            Zone::Center => {
                main.split_right(NodeIndex::root(), 0.3, vec![pane]);
            }
        }
    }
    if let Some(path) = dock.find_tab(&pane) {
        let _ = dock.focus_tab(path);
    }
}

pub fn toggle(dock: &mut DockState<Pane>, pane: Pane, floats: &Floats) {
    if has(dock, pane) {
        hide(dock, pane);
    } else {
        show(dock, pane, floats);
    }
}

#[cfg(test)]
pub fn sync(dock: &mut DockState<Pane>, docs: &[u64]) {
    sync_with(dock, docs, docs.is_empty());
}

pub fn sync_with(dock: &mut DockState<Pane>, docs: &[u64], welcome: bool) {
    if welcome && !has(dock, Pane::Welcome) {
        show(dock, Pane::Welcome, &Vec::new());
    }
    for id in docs {
        if !has(dock, Pane::Doc(*id)) {
            show(dock, Pane::Doc(*id), &Vec::new());
        }
    }
    let stale: Vec<Pane> = dock
        .iter_all_tabs()
        .map(|(_, tab)| *tab)
        .filter(|tab| match tab {
            Pane::Doc(id) => !docs.contains(id),
            Pane::Welcome => !welcome,
            _ => false,
        })
        .collect();
    for pane in stale {
        hide(dock, pane);
    }
}

#[derive(Clone, Copy, Default)]
pub struct Mark {
    pub dirty: bool,
    pub pinned: bool,
    pub accent: Color32,
}

thread_local! {
    static MARK: std::cell::Cell<Mark> = const {
        std::cell::Cell::new(Mark {
            dirty: false,
            pinned: false,
            accent: Color32::TRANSPARENT,
        })
    };
}

pub fn set_mark(mark: Mark) {
    MARK.with(|m| m.set(mark));
}

pub fn pins_first(dock: &mut DockState<Pane>, pinned: &[u64]) {
    if pinned.is_empty() {
        return;
    }
    let is_pinned = |tab: &Pane| matches!(tab, Pane::Doc(id) if pinned.contains(id));
    let paths: Vec<NodePath> = dock.iter_leaves().map(|(path, _)| path).collect();
    for path in paths {
        let Ok(leaf) = dock.leaf_mut(path) else {
            continue;
        };
        if leaf.tabs.iter().skip_while(|t| is_pinned(t)).all(|t| !is_pinned(t)) {
            continue;
        }
        let active = leaf.tabs.get(leaf.active.0).copied();
        leaf.tabs.sort_by_key(|tab| !is_pinned(tab));
        if let Some(index) = active.and_then(|a| leaf.tabs.iter().position(|t| *t == a)) {
            leaf.active = egui_stack::TabIndex(index);
        }
    }
}

pub struct TabPainter;

impl egui_stack::DockPainter for TabPainter {
    fn tab(&self, painter: &egui::Painter, tab: &egui_stack::TabPaint<'_>) {
        egui_stack::DockPainter::tab(&egui_stack::StylePainter, painter, tab);
        let Some(close) = tab.close else {
            return;
        };
        let mark = MARK.with(|m| m.get());
        let center = close.rect.center();
        let lit = close.state.contains(egui_stack::PartState::HOVERED)
            || close.state.contains(egui_stack::PartState::FOCUSED);
        let shown = tab.state.contains(egui_stack::PartState::HOVERED)
            || tab.state.contains(egui_stack::PartState::SELECTED);
        let text = tab.tab_style.active.text_color;
        if lit {
            let inner = egui::Rect::from_center_size(
                center,
                egui::vec2(18.0, 18.0).min(tab.rect.size() - egui::vec2(4.0, 6.0)),
            )
            .intersect(tab.rect.shrink2(egui::vec2(2.0, 3.0)));
            painter.rect_filled(inner, 5.0, tab.style.buttons.add_tab_bg_fill);
        }
        if mark.pinned {
            let color = if lit { text } else { mark.accent };
            crate::widgets::paint_pin(painter, center, color);
        } else if mark.dirty && !lit {
            painter.circle_filled(center, 6.5, mark.accent.gamma_multiply(0.18));
            painter.circle_filled(center, 3.5, mark.accent);
        } else if lit || shown {
            let d = 3.5;
            let color = if lit {
                text
            } else {
                tab.tab_style.inactive.text_color.gamma_multiply(0.75)
            };
            let stroke = Stroke::new(1.3, color);
            painter.line_segment([center + egui::vec2(-d, -d), center + egui::vec2(d, d)], stroke);
            painter.line_segment([center + egui::vec2(d, -d), center + egui::vec2(-d, d)], stroke);
        }
    }
}

pub static TAB_PAINTER: TabPainter = TabPainter;

pub fn leaf_min(tabs: &[Pane]) -> egui::Vec2 {
    tabs.iter().fold(egui::Vec2::ZERO, |acc, tab| {
        let (w, h) = tab.min_size();
        acc.max(egui::vec2(w, h + 34.0))
    })
}

pub fn enforce_min(dock: &mut DockState<Pane>) {
    let surfaces: Vec<SurfaceIndex> = dock.iter_surfaces_indexed().map(|(i, _)| i).collect();
    for surface in surfaces {
        let Some(tree) = dock.get_surface_mut(surface).and_then(|s| s.node_tree_mut()) else {
            continue;
        };
        let leaves: Vec<(NodeIndex, egui::Rect, egui::Vec2)> = tree
            .iter()
            .enumerate()
            .filter_map(|(i, node)| {
                let leaf = node.get_leaf()?;
                Some((NodeIndex(i), leaf.rect(), leaf_min(leaf.tabs())))
            })
            .collect();
        for (node, rect, min) in leaves {
            if !rect.is_finite() || rect.width() <= 1.0 {
                continue;
            }
            for (horizontal, have, need) in [
                (true, rect.width(), min.x),
                (false, rect.height(), min.y),
            ] {
                if have + 3.0 >= need {
                    continue;
                }
                let mut child = node;
                while let Some(parent) = tree.parent(child) {
                    let Some(linear) = tree[parent].get_linear_mut() else {
                        child = parent;
                        continue;
                    };
                    let along = matches!(linear.dir, egui_stack::LinearDir::Horizontal) == horizontal;
                    if !along || linear.len() < 2 {
                        child = parent;
                        continue;
                    }
                    let Some(index) = linear.position(child) else {
                        break;
                    };
                    let span = if horizontal {
                        linear.rect().width()
                    } else {
                        linear.rect().height()
                    };
                    if span <= need * 1.1 {
                        break;
                    }
                    let shares = linear.shares().to_vec();
                    let total: f32 = shares.iter().sum();
                    let wanted = (need + 2.0) / span * total;
                    let current = shares[index];
                    if wanted > current {
                        let rest = total - current;
                        let left = (total - wanted).max(total * 0.1);
                        let scale = if rest > 0.0 { left / rest } else { 1.0 };
                        let updated: Vec<f32> = shares
                            .iter()
                            .enumerate()
                            .map(|(i, s)| if i == index { total - left } else { s * scale })
                            .collect();
                        linear.set_shares(updated);
                    }
                    break;
                }
            }
        }
    }
}

pub fn style(pal: &Palette, egui_style: &egui::Style, radius: u8) -> Style {
    let mut style = Style::from_egui(egui_style);
    let round = CornerRadius {
        nw: radius,
        ne: radius,
        sw: 0,
        se: 0,
    };
    style.dock_area_padding = None;
    style.main_surface_border_stroke = Stroke::NONE;

    style.separator.width = 1.0;
    style.separator.extra_interact_width = 6.0;
    style.separator.color_idle = pal.border;
    style.separator.color_hovered = theme::blend(pal.border, pal.accent, 0.45);
    style.separator.color_dragged = theme::blend(pal.border, pal.accent, 0.8);

    style.tab_bar.bg_fill = pal.mantle;
    style.tab_bar.height = 34.0;
    style.tab_bar.hline_color = pal.border;
    style.tab_bar.corner_radius = CornerRadius::ZERO;
    style.tab_bar.fill_tab_bar = false;
    style.tab_bar.show_scroll_bar_on_overflow = false;
    style.tab_bar.inner_margin = Margin {
        left: 6,
        right: 6,
        top: 4,
        bottom: 0,
    };

    let state = |fill: Color32, text: Color32, outline: Color32| egui_stack::TabInteractionStyle {
        outline_color: outline,
        corner_radius: round,
        bg_fill: fill,
        text_color: text,
    };
    style.tab.active = state(pal.base, pal.text, pal.border);
    style.tab.focused = state(pal.base, pal.text, pal.border);
    style.tab.inactive = state(Color32::TRANSPARENT, pal.subtext, Color32::TRANSPARENT);
    style.tab.hovered = state(pal.crust, pal.text, Color32::TRANSPARENT);
    style.tab.active_with_kb_focus = style.tab.active.clone();
    style.tab.focused_with_kb_focus = style.tab.focused.clone();
    style.tab.inactive_with_kb_focus = style.tab.inactive.clone();
    style.tab.hline_below_active_tab_name = false;
    style.tab.spacing = 2.0;
    style.tab.tab_body.inner_margin = Margin::ZERO;
    style.tab.tab_body.stroke = Stroke::NONE;
    style.tab.tab_body.corner_radius = CornerRadius::ZERO;
    style.tab.tab_body.bg_fill = pal.mantle;

    style.buttons.close_tab_color = Color32::TRANSPARENT;
    style.buttons.close_tab_active_color = Color32::TRANSPARENT;
    style.buttons.close_tab_bg_fill = Color32::TRANSPARENT;
    style.buttons.add_tab_bg_fill = pal.surface_hover;

    style.overlay.selection_color = pal.accent.gamma_multiply(0.22);
    style.overlay.button_color = pal.accent.gamma_multiply(0.6);
    style.overlay.button_border_stroke = Stroke::new(1.0, pal.accent);
    style
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zones(dock: &DockState<Pane>) -> Vec<Vec<Pane>> {
        dock.iter_leaves()
            .map(|(_, leaf)| leaf.tabs().to_vec())
            .collect()
    }

    #[test]
    fn panes_land_in_their_zones_and_group_together() {
        let mut dock = new();
        sync(&mut dock, &[1, 2]);
        assert_eq!(zones(&dock), [vec![Pane::Doc(1), Pane::Doc(2)]]);
        show(&mut dock, Pane::Hosts, &Vec::new());
        show(&mut dock, Pane::Console, &Vec::new());
        show(&mut dock, Pane::Log, &Vec::new());
        show(&mut dock, Pane::Settings, &Vec::new());
        let mut leaves = zones(&dock);
        leaves.sort_by_key(|l| format!("{l:?}"));
        assert_eq!(
            leaves,
            [
                vec![Pane::Console, Pane::Log],
                vec![Pane::Doc(1), Pane::Doc(2), Pane::Settings],
                vec![Pane::Hosts],
            ]
        );
        toggle(&mut dock, Pane::Hosts, &Vec::new());
        assert!(!has(&dock, Pane::Hosts));
        sync(&mut dock, &[]);
        assert!(has(&dock, Pane::Welcome) && !has(&dock, Pane::Doc(1)));
        let welcome = dock.find_tab(&Pane::Welcome).unwrap();
        let settings = dock.find_tab(&Pane::Settings).unwrap();
        assert_eq!(welcome.node, settings.node);
        sync(&mut dock, &[7]);
        assert!(!has(&dock, Pane::Welcome));
        assert_eq!(dock.find_tab(&Pane::Doc(7)).unwrap().node, settings.node);
    }

    #[test]
    fn hidden_pane_returns_to_its_place() {
        set_homes(Vec::new());
        let mut dock = new();
        sync(&mut dock, &[1]);
        show(&mut dock, Pane::Hosts, &Vec::new());
        show(&mut dock, Pane::Project, &Vec::new());
        let project = dock.find_tab(&Pane::Project).unwrap();
        let hosts = dock.find_tab(&Pane::Hosts).unwrap();
        assert_eq!(project.node, hosts.node);
        remember_homes(&dock);
        hide(&mut dock, Pane::Project);
        show(&mut dock, Pane::Log, &Vec::new());
        show(&mut dock, Pane::Project, &Vec::new());
        let back = dock.find_tab(&Pane::Project).unwrap();
        assert_eq!(back.node, dock.find_tab(&Pane::Hosts).unwrap().node);
        let log = dock.find_tab(&Pane::Log).unwrap();
        remember_homes(&dock);
        let before = dock.main_surface()[log.node].rect();
        hide(&mut dock, Pane::Log);
        show(&mut dock, Pane::Log, &Vec::new());
        let again = dock.find_tab(&Pane::Log).unwrap();
        assert_ne!(again.node, back.node);
        assert_eq!(dock.main_surface()[again.node].rect().is_some(), before.is_some());
    }

    #[test]
    fn layout_survives_serialization() {
        let mut dock = new();
        sync(&mut dock, &[3]);
        show(&mut dock, Pane::Hosts, &Vec::new());
        show(&mut dock, Pane::Log, &Vec::new());
        let text = ron::ser::to_string(&dock).unwrap();
        let back: DockState<Pane> = ron::from_str(&text).unwrap();
        assert_eq!(zones(&back), zones(&dock));
    }

    #[test]
    fn pinned_tabs_move_to_the_front_and_keep_their_order() {
        let mut dock = DockState::new(vec![
            Pane::Doc(1),
            Pane::Doc(2),
            Pane::Doc(3),
            Pane::Doc(4),
        ]);
        let path = dock.iter_leaves().next().unwrap().0;
        dock.leaf_mut(path).unwrap().active = egui_stack::TabIndex(1);
        pins_first(&mut dock, &[4, 3]);
        let leaf = dock.leaf_mut(path).unwrap();
        assert_eq!(
            leaf.tabs,
            vec![Pane::Doc(3), Pane::Doc(4), Pane::Doc(1), Pane::Doc(2)]
        );
        assert_eq!(leaf.tabs[leaf.active.0], Pane::Doc(2));
    }
}
