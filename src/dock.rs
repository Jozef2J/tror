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
            Pane::Hosts | Pane::Hosts2 | Pane::HostsN(_) | Pane::Project => (170.0, 120.0),
            Pane::Log => (200.0, 80.0),
            Pane::Console => (240.0, 100.0),
            Pane::ServerSearch => (240.0, 100.0),
            Pane::Db => (300.0, 160.0),
            Pane::Settings | Pane::Diff => (320.0, 160.0),
            Pane::Preview => (200.0, 120.0),
        }
    }

    pub fn title(self) -> String {
        match self {
            Pane::Hosts2 => "Подключение 2".into(),
            Pane::HostsN(n) => format!("Подключение {}", n as usize + 1),
            other => other.label().into(),
        }
    }

    fn label(self) -> &'static str {
        match self {
            Pane::Welcome => "Tror",
            Pane::Doc(_) => "",
            Pane::Hosts => "Удалённые хосты",
            Pane::Hosts2 | Pane::HostsN(_) => "",
            Pane::Project => "Проект",
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
            Pane::Hosts | Pane::Hosts2 | Pane::HostsN(_) | Pane::Project => Zone::Left,
            Pane::Console | Pane::Log | Pane::ServerSearch => Zone::Bottom,
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
    t.tab_context_menu.eject_button = "Открепить (плавающая панель)".into();
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

pub fn remember(dock: &DockState<Pane>, floats: &mut Floats) {
    for (path, tab) in dock.iter_all_tabs() {
        if matches!(tab, Pane::Doc(_) | Pane::Welcome) {
            continue;
        }
        floats.retain(|(pane, _)| pane != tab);
        if path.surface == SurfaceIndex::main() {
            continue;
        }
        if let Some(rect) = dock.get_window_state(path.surface).map(|w| w.rect())
            && rect.is_finite()
            && rect.width() > 40.0
            && rect.height() > 40.0
        {
            floats.push((*tab, [rect.min.x, rect.min.y, rect.width(), rect.height()]));
        }
    }
}

pub fn show(dock: &mut DockState<Pane>, pane: Pane, floats: &Floats) {
    if let Some(path) = dock.find_tab(&pane) {
        let _ = dock.focus_tab(path);
        return;
    }
    if let Some((_, [x, y, w, h])) = floats.iter().find(|(p, _)| *p == pane) {
        let surface = dock.add_window(vec![pane]);
        if let Some(window) = dock.get_window_state_mut(surface) {
            window.set_position(egui::pos2(*x, *y));
            window.set_size(egui::vec2(*w, *h));
        }
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

pub struct TabPainter;

impl egui_stack::DockPainter for TabPainter {
    fn tab(&self, painter: &egui::Painter, tab: &egui_stack::TabPaint<'_>) {
        egui_stack::DockPainter::tab(&egui_stack::StylePainter, painter, tab);
        let Some(close) = tab.close else {
            return;
        };
        let lit = close.state.contains(egui_stack::PartState::HOVERED)
            || close.state.contains(egui_stack::PartState::FOCUSED);
        if !lit {
            return;
        }
        let inner = egui::Rect::from_center_size(
            close.rect.center(),
            egui::vec2(18.0, 18.0).min(tab.rect.size() - egui::vec2(4.0, 6.0)),
        )
        .intersect(tab.rect.shrink2(egui::vec2(2.0, 3.0)));
        painter.rect_filled(inner, 4.0, tab.style.buttons.add_tab_bg_fill);
        let d = 4.0;
        let c = inner.center();
        let stroke = Stroke::new(1.2, tab.style.buttons.close_tab_active_color);
        painter.line_segment([c + egui::vec2(-d, -d), c + egui::vec2(d, d)], stroke);
        painter.line_segment([c + egui::vec2(d, -d), c + egui::vec2(-d, d)], stroke);
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
                if have + 0.5 >= need {
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

    style.buttons.close_tab_color = pal.faint;
    style.buttons.close_tab_active_color = pal.text;
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
    fn hidden_floating_pane_returns_to_its_window() {
        let mut dock = new();
        sync(&mut dock, &[1]);
        let mut floats = Floats::new();
        show(&mut dock, Pane::Log, &floats);
        let path = dock.find_tab(&Pane::Log).unwrap();
        let rect = egui::Rect::from_min_size(egui::pos2(300.0, 200.0), egui::vec2(420.0, 260.0));
        dock.detach_tab(path, rect);
        let surface = dock.find_tab(&Pane::Log).unwrap().surface;
        assert_ne!(surface, SurfaceIndex::main());
        floats.push((Pane::Log, [300.0, 200.0, 420.0, 260.0]));
        hide(&mut dock, Pane::Log);
        assert!(!has(&dock, Pane::Log));
        show(&mut dock, Pane::Log, &floats);
        let surface = dock.find_tab(&Pane::Log).unwrap().surface;
        assert_ne!(surface, SurfaceIndex::main());
        show(&mut dock, Pane::Hosts, &floats);
        assert_eq!(
            dock.find_tab(&Pane::Hosts).unwrap().surface,
            SurfaceIndex::main()
        );
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
}
