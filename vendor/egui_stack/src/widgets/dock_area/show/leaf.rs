use egui::{
    Align, Button, Color32, CornerRadius, CursorIcon, EventFilter, Frame, Id, Key, LayerId, Layout,
    NumExt, Order, Popup, PopupCloseBehavior, Rect, Response, ScrollArea, Sense, Shape, Stroke,
    TextStyle, Ui, UiBuilder, Vec2, WidgetInfo, WidgetText, accesskit::Role, emath::TSTransform,
    lerp, pos2, vec2,
};

use crate::NodePath;
use crate::dock_area::tab_removal::{ForcedRemoval, TabRemoval};
use crate::node::LeafNode;
use crate::tab_viewer::OnCloseResponse;
use crate::{
    DockArea, DraggedTab, NodeIndex, PartState, Style, SurfaceIndex, TabAddAlign, TabBarPaint,
    TabBarTailPaint, TabBodyPaint, TabClose, TabFocus, TabIndex, TabMeasure, TabPaint, TabStyle,
    TabViewer,
    dock_area::{
        drag_and_drop::{DragData, DragDropState, HoverData, TreeComponent},
        state::State,
    },
    utils::{fade_visuals, rect_set_size_centered},
    widgets::painter::{Attached, corners_within},
};

fn tab_body_id(dock_area_id: Id, path: NodePath, tab_id: Id) -> Id {
    dock_area_id.with((path.surface, "surface")).with(tab_id)
}

impl<Tab> DockArea<'_, Tab> {
    pub(super) fn show_leaf(
        &mut self,
        ui: &mut Ui,
        state: &mut State,
        path: NodePath,
        tab_viewer: &mut impl TabViewer<Tab = Tab>,
        fade_style: Option<(&Style, f32)>,
    ) {
        assert!(self.dock_state[path].is_leaf());
        let collapsed = self.dock_state[path].is_collapsed();

        let rect = self.dock_state[path]
            .rect()
            .expect("This node must be a leaf");

        if rect.width() <= 0.0 || rect.height() <= 0.0 {
            return;
        }

        let ui = &mut ui.new_child(
            UiBuilder::new()
                .max_rect(rect)
                .layout(Layout::top_down_justified(Align::Min))
                .id_salt((path.node, "node")),
        );
        let spacing = ui.spacing().item_spacing;
        ui.spacing_mut().item_spacing = Vec2::ZERO;
        ui.set_clip_rect(rect);

        if self.dock_state[path].tabs_count() == 0 {
            return;
        }

        let leaf = self
            .dock_state
            .leaf(path)
            .expect("This node must be a leaf");
        let tab_bar_hidden = self.hidable_tab_bars && leaf.tab_bar_hidden;
        let style = fade_style.map(|(style, _)| style);

        let tabbar_rect = if !tab_bar_hidden {
            self.tab_bar(ui, state, path, tab_viewer, style, collapsed)
        } else {
            // use the top margin of the tab body as a tab bar
            let style = style.unwrap_or_else(|| self.style.as_ref().unwrap());
            let drag_height = style
                .tab
                .tab_body
                .hidden_tab_bar_drag_height
                .unwrap_or(style.tab.tab_body.inner_margin.top as f32);
            let drag_size = vec2(ui.available_width(), drag_height);
            Rect::from_min_size(ui.cursor().min, drag_size)
        };
        self.tab_body(
            ui,
            state,
            path,
            tab_viewer,
            spacing,
            tabbar_rect,
            fade_style,
            collapsed,
        );

        if tab_bar_hidden {
            self.hidden_tab_bar_controls(ui, state, path, tabbar_rect, style);
        }

        let tabs = self.dock_state[path]
            .tabs_mut()
            .expect("This node must be a leaf here");
        for (tab_index, tab) in tabs.iter_mut().enumerate() {
            if tab_viewer.force_close(tab) {
                self.to_remove.push(TabRemoval::Tab(
                    (path, TabIndex(tab_index)).into(),
                    ForcedRemoval(true),
                ));
            }
        }
    }

    /// Draws controls for the hidden tab bar: a triangle button to show it,
    /// a right-click context menu, and drag handling on the top margin.
    fn hidden_tab_bar_controls(
        &mut self,
        ui: &mut Ui,
        state: &mut State,
        path: NodePath,
        tabbar_rect: Rect,
        fade_style: Option<&Style>,
    ) {
        let style = fade_style.unwrap_or_else(|| self.style.as_ref().unwrap());
        let btn_size = style.buttons.show_tab_bar_size;
        let btn_expand = style.buttons.show_tab_bar_hover_expand;
        let btn_color = style.buttons.show_tab_bar_color;
        let btn_active_color = style.buttons.show_tab_bar_active_color;

        // register the drag area first so the button takes priority over it
        let drag_id = self
            .id
            .with((path.surface, "surface"))
            .with((path.node, "node"))
            .with("hidden_tab_bar_drag");
        let drag_response = ui.interact(tabbar_rect, drag_id, Sense::click_and_drag());

        // triangle button in the top-left corner to show the tab bar
        let btn_rect = Rect::from_min_size(tabbar_rect.left_top(), Vec2::splat(btn_size));
        let btn_id = self
            .id
            .with((path.surface, "surface"))
            .with((path.node, "node"))
            .with("show_tab_bar_btn");
        let btn_response = ui.interact(btn_rect, btn_id, Sense::click());
        let enabled = ui.is_enabled();
        btn_response
            .widget_info(|| WidgetInfo::labeled(Role::Button, enabled, &self.labels.show_tab_bar));
        let (draw_rect, color) = if btn_response.hovered() {
            let expanded_size = Vec2::splat(btn_size + btn_expand);
            (
                Rect::from_min_size(btn_rect.left_top(), expanded_size),
                btn_active_color,
            )
        } else {
            (btn_rect, btn_color)
        };
        ui.painter().add(Shape::convex_polygon(
            vec![
                draw_rect.left_top(),
                draw_rect.right_top(),
                draw_rect.left_bottom(),
            ],
            color,
            Stroke::NONE,
        ));
        if btn_response.clicked() {
            self.dock_state[path]
                .get_leaf_mut()
                .expect("This node must be a leaf")
                .tab_bar_hidden = false;
        }

        let on_button = btn_response.hovered() || btn_response.clicked();

        // right-click context menu to show the tab bar
        if self.tab_context_menus && self.show_builtin_tab_menu && !on_button {
            let show_button = Button::new(
                &self
                    .dock_state
                    .translations
                    .tab_context_menu
                    .show_tab_bar_button,
            );
            drag_response.context_menu(|ui| {
                if ui.add(show_button).clicked() {
                    self.dock_state[path]
                        .get_leaf_mut()
                        .expect("This node must be a leaf")
                        .tab_bar_hidden = false;
                    ui.close();
                }
            });
        }

        // allow tab to be draggable via the top margin of the tab body

        let drag_enabled = style
            .tab
            .tab_body
            .hidden_tab_bar_drag_height
            .is_none_or(|h| h > 0.0);
        if self.draggable_tabs && drag_enabled && !on_button {
            if drag_response.hovered() {
                ui.output_mut(|o| o.cursor_icon = CursorIcon::Grab);
            }
            let is_being_dragged = ui.ctx().is_being_dragged(drag_id)
                && ui.input(|i| i.pointer.is_decidedly_dragging());
            if is_being_dragged {
                ui.output_mut(|o| o.cursor_icon = CursorIcon::Grabbing);
                let active = self.dock_state[path]
                    .get_leaf()
                    .expect("This node must be a leaf")
                    .active;
                self.try_initiate_tab_drag(ui, state, path, active);
            }
        }
    }

    fn tab_bar(
        &mut self,
        ui: &mut Ui,
        state: &mut State,
        path: NodePath,
        tab_viewer: &mut impl TabViewer<Tab = Tab>,
        fade_style: Option<&Style>,
        collapsed: bool,
    ) -> Rect {
        assert!(self.dock_state[path].is_leaf());

        let style = fade_style.unwrap_or_else(|| self.style.as_ref().unwrap());
        let (tabbar_outer_rect, tabbar_response) = ui.allocate_exact_size(
            vec2(ui.available_width(), style.tab_bar.height),
            Sense::hover(),
        );
        let scrolled = 0.0 - self.dock_state.leaf(path).map_or(0.0, |leaf| leaf.scroll);
        // The bar's outline, which the highlights of its buttons keep to at its corners.
        let attached = if collapsed {
            Attached::Nothing
        } else {
            Attached::Below
        };
        let bar = (
            tabbar_outer_rect,
            attached.corner_radius(style.tab_bar.corner_radius),
        );
        self.painter.tab_bar(
            ui.painter(),
            &TabBarPaint {
                rect: tabbar_outer_rect,
                scrolled,
                folded: collapsed,
                style,
            },
        );

        let tabbar_outer_rect = tabbar_outer_rect - style.tab_bar.inner_margin;

        let mut available_width = tabbar_outer_rect.width();
        let scroll_bar_width = available_width;
        if available_width == 0.0 {
            return tabbar_outer_rect;
        }

        // Reserve space for the buttons at the ends of the tab bar.

        if self.show_add_buttons {
            available_width -= Style::TAB_ADD_BUTTON_SIZE;
        }

        if self.show_leaf_close_all_buttons {
            available_width -= Style::TAB_CLOSE_ALL_BUTTON_SIZE;
        }

        if self.show_leaf_collapse_buttons {
            available_width -= Style::TAB_COLLAPSE_BUTTON_SIZE;
        }

        let (actual_width, tab_hovered) = {
            let leaf = self
                .dock_state
                .leaf_mut(path)
                .expect("This node must be a leaf");

            let tabbar_inner_rect = Rect::from_min_size(
                (tabbar_outer_rect.min - pos2(-leaf.scroll, 0.0)
                    + vec2(
                        if self.show_leaf_collapse_buttons {
                            Style::TAB_COLLAPSE_BUTTON_SIZE
                        } else {
                            0.0
                        },
                        0.0,
                    ))
                .to_pos2(),
                vec2(tabbar_outer_rect.width(), tabbar_outer_rect.height()),
            );

            let tabs_ui = &mut ui.new_child(
                UiBuilder::new()
                    .max_rect(tabbar_inner_rect)
                    .layout(Layout::left_to_right(Align::Center))
                    .id_salt("tabs")
                    .accessibility_role(Role::TabList),
            );

            let mut clip_rect = tabbar_outer_rect;
            clip_rect.set_width(available_width);
            if self.show_leaf_collapse_buttons {
                clip_rect = clip_rect.translate(vec2(Style::TAB_COLLAPSE_BUTTON_SIZE, 0.0));
            }
            tabs_ui.set_clip_rect(clip_rect);

            // Desired size for tabs in "expanded" mode.
            let prefered_width = style
                .tab_bar
                .fill_tab_bar
                .then_some(available_width / (leaf.tabs.len() as f32));

            let tab_hovered = self.tabs(
                tabs_ui,
                state,
                path,
                tab_viewer,
                tabbar_outer_rect,
                prefered_width,
                fade_style,
                collapsed,
            );

            // The rest of the bar, right of the tabs.
            let style = fade_style.unwrap_or_else(|| self.style.as_ref().unwrap());
            let tail = Rect::from_min_max(
                pos2(
                    tabs_ui.min_rect().right().min(clip_rect.right()),
                    tabbar_outer_rect.top(),
                ),
                tabbar_outer_rect.right_bottom(),
            );
            self.painter.tab_bar_tail(
                ui.painter(),
                &TabBarTailPaint {
                    rect: tail,
                    bar: tabbar_outer_rect,
                    folded: collapsed,
                    style,
                },
            );

            // Add button at the ends of the tab bar.
            if self.show_add_buttons {
                let offset = match style.buttons.add_tab_align {
                    TabAddAlign::Left => {
                        (clip_rect.width() - tabs_ui.min_rect().width()).at_least(0.0)
                    }
                    TabAddAlign::Right => 0.0,
                } + if self.show_leaf_close_all_buttons {
                    Style::TAB_CLOSE_ALL_BUTTON_SIZE
                } else {
                    0.0
                };
                self.tab_plus(
                    ui,
                    path,
                    tab_viewer,
                    tabbar_outer_rect,
                    offset,
                    fade_style,
                    bar,
                );
            }

            if self.show_leaf_close_all_buttons {
                // Current leaf contains non-closable tabs.
                let disabled = self
                    .dock_state
                    .leaf_mut(path)
                    .map(|leaf| !leaf.tabs.iter_mut().all(|tab| tab_viewer.is_closeable(tab)))
                    .expect("This node must be a leaf");

                // Current window contains non-closable tabs.
                let close_window_disabled = disabled
                    || !self.dock_state[path.surface].iter_mut().all(|node| {
                        node.get_leaf_mut().is_none_or(|leaf| {
                            leaf.tabs.iter_mut().all(|tab| tab_viewer.is_closeable(tab))
                        })
                    });

                self.tab_close_all(
                    ui,
                    path,
                    tabbar_outer_rect,
                    fade_style,
                    disabled,
                    close_window_disabled,
                    bar,
                )
            }

            if self.show_leaf_collapse_buttons {
                self.tab_collapse(ui, path, tabbar_outer_rect, fade_style, collapsed, bar)
            }

            (tabs_ui.min_rect().width(), tab_hovered)
        };

        self.tab_bar_scroll(
            ui,
            state,
            path,
            actual_width,
            available_width,
            scroll_bar_width,
            &tabbar_response,
            tab_hovered,
            fade_style,
        );

        tabbar_outer_rect
    }

    #[allow(clippy::too_many_arguments)]
    fn tabs(
        &mut self,
        tabs_ui: &mut Ui,
        state: &mut State,
        path: NodePath,
        tab_viewer: &mut impl TabViewer<Tab = Tab>,
        tabbar_outer_rect: Rect,
        preferred_width: Option<f32>,
        fade: Option<&Style>,
        folded: bool,
    ) -> bool {
        let mut tab_hovered = false;

        assert!(self.dock_state[path].is_leaf());

        let focused = self.dock_state.focused_leaf();
        let tabs_len = {
            let tabs = self.dock_state[path]
                .tabs()
                .expect("This node must be a leaf here");
            tabs.len()
        };

        let mut roved = false;
        for tab_index in 0..tabs_len {
            let id = self
                .id
                .with((path.surface, "surface"))
                .with((path.node, "node"))
                .with((tab_index, "tab"));
            let tab_index = TabIndex(tab_index);
            let is_being_dragged = tabs_ui.ctx().is_being_dragged(id)
                && tabs_ui.input(|i| i.pointer.is_decidedly_dragging())
                && self.draggable_tabs;

            if is_being_dragged {
                tabs_ui.output_mut(|o| o.cursor_icon = CursorIcon::Grabbing);
            }

            let (selected, label, tab_style, closeable) = {
                let leaf = self.dock_state[path]
                    .get_leaf_mut()
                    .expect("This node must be a leaf");
                let style = fade.unwrap_or_else(|| self.style.as_ref().unwrap());
                let tab_style = tab_viewer.tab_style_override(&leaf.tabs[tab_index.0], &style.tab);
                (
                    leaf.active == tab_index,
                    tab_viewer.title(&mut leaf.tabs[tab_index.0]),
                    tab_style.unwrap_or(style.tab.clone()),
                    tab_viewer.is_closeable(&leaf.tabs[tab_index.0]),
                )
            };

            let show_close_button = self.show_close_buttons && closeable;
            let flags = PartState::NONE
                .with(PartState::SELECTED, selected)
                .with(PartState::DRAGGED, is_being_dragged)
                .with(PartState::LEAF_FOCUSED, Some(path) == focused);

            let (response, title_id) = if is_being_dragged {
                let layer_id = LayerId::new(Order::Tooltip, id);
                let response = tabs_ui
                    .scope_builder(UiBuilder::new().layer_id(layer_id), |ui| {
                        self.tab_title(
                            ui,
                            &tab_style,
                            id,
                            label,
                            flags,
                            tabbar_outer_rect,
                            preferred_width,
                            show_close_button,
                            fade,
                            folded,
                        )
                    })
                    .response;
                let title_id = response.id;

                let response =
                    tabs_ui.interact(response.rect, id.with("dragged"), Sense::click_and_drag());

                if let Some(delta) = self.try_initiate_tab_drag(tabs_ui, state, path, tab_index) {
                    tabs_ui
                        .ctx()
                        .transform_layer_shapes(layer_id, TSTransform::new(delta, 1.0));
                }

                (response, title_id)
            } else {
                if tab_index.0 != 0 {
                    tabs_ui.allocate_space(vec2(tab_style.spacing, 0.0));
                }
                let (mut response, close_response) = self.tab_title(
                    tabs_ui,
                    &tab_style,
                    id,
                    label,
                    flags,
                    tabbar_outer_rect,
                    preferred_width,
                    show_close_button,
                    fade,
                    folded,
                );
                let title_id = response.id;
                let close_clicked = close_response.is_some_and(|res| res.clicked());
                let is_lonely_tab = self.dock_state[path.surface].num_tabs() == 1;

                if self.show_tab_name_on_hover {
                    let tabs = self.dock_state[path]
                        .tabs_mut()
                        .expect("This node must be a leaf");
                    let tab = &mut tabs[tab_index.0];
                    response = response.on_hover_ui(|ui| {
                        ui.label(tab_viewer.title(tab));
                    });
                }

                if self.tab_context_menus {
                    let eject_button =
                        Button::new(&self.dock_state.translations.tab_context_menu.eject_button);
                    let close_button =
                        Button::new(&self.dock_state.translations.tab_context_menu.close_button);
                    let hide_tab_bar_button = Button::new(
                        &self
                            .dock_state
                            .translations
                            .tab_context_menu
                            .hide_tab_bar_button,
                    );

                    let in_window = self.dock_state.is_window(path.surface);
                    let may_float = self.may_float((path, tab_index).into(), tab_viewer);
                    response.context_menu(|ui| {
                        let leaf = self.dock_state[path]
                            .get_leaf_mut()
                            .expect("This node must be a leaf");
                        let tab = &mut leaf.tabs[tab_index.0];

                        tab_viewer.context_menu(ui, tab, (path, tab_index).into());
                        if !self.show_builtin_tab_menu {
                            return;
                        }
                        if (!in_window || !is_lonely_tab)
                            && may_float
                            && ui.add(eject_button).clicked()
                        {
                            self.to_detach.push((path, tab_index).into());
                            ui.close();
                        }
                        if show_close_button
                            && tab_viewer.close_in_tab_menu(tab)
                            && ui.add(close_button).clicked()
                        {
                            match tab_viewer.on_close(tab) {
                                // The viewer already agreed: remove without asking again.
                                OnCloseResponse::Close => self.to_remove.push(TabRemoval::Tab(
                                    (path, tab_index).into(),
                                    ForcedRemoval(true),
                                )),
                                OnCloseResponse::Focus => {
                                    leaf.active = tab_index;
                                    self.new_focused = Some(path);
                                }
                                OnCloseResponse::Ignore => (),
                            }
                            ui.close();
                        }
                        if self.hidable_tab_bars && ui.add(hide_tab_bar_button).clicked() {
                            leaf.tab_bar_hidden = true;
                        }
                    });
                }

                if close_clicked {
                    self.to_remove.push(TabRemoval::Tab(
                        (path, tab_index).into(),
                        ForcedRemoval(false),
                    ));
                }

                if let Some(pos) = state.last_hover_pos {
                    // Use response.rect.contains instead of
                    // response.hovered as the dragged tab covers
                    // the underlying tab
                    if state.drag_start.is_some() && response.rect.contains(pos) {
                        self.tab_hover_rect = Some((response.rect, tab_index));
                    }
                }

                (response, title_id)
            };

            if response.hovered() {
                tab_hovered = true;
            }

            let leaf = self.dock_state.leaf_mut(path).unwrap();
            let tab = &mut leaf.tabs[tab_index.0];

            if response.clicked()
                || (tabs_ui.memory(|m| m.has_focus(title_id))
                    && tabs_ui.input(|i| i.key_pressed(Key::Enter) || i.key_pressed(Key::Space)))
            {
                leaf.active = tab_index;
                self.new_focused = Some(path);
            }

            // Roving focus: the arrows, Home and End select and focus another tab of the bar.
            // The focus moves at once, so the tab it lands on must not act on the same key.
            if self.tab_focus == TabFocus::Roving
                && !roved
                && tabs_ui.memory(|m| m.has_focus(title_id))
            {
                tabs_ui.memory_mut(|m| {
                    m.set_focus_lock_filter(
                        title_id,
                        EventFilter {
                            horizontal_arrows: true,
                            vertical_arrows: false,
                            tab: false,
                            escape: false,
                        },
                    )
                });
                let target = tabs_ui.input(|i| {
                    if i.key_pressed(Key::ArrowRight) {
                        Some((tab_index.0 + 1) % tabs_len)
                    } else if i.key_pressed(Key::ArrowLeft) {
                        Some((tab_index.0 + tabs_len - 1) % tabs_len)
                    } else if i.key_pressed(Key::Home) {
                        Some(0)
                    } else if i.key_pressed(Key::End) {
                        Some(tabs_len - 1)
                    } else {
                        None
                    }
                });
                if let Some(target) = target
                    && target != tab_index.0
                {
                    leaf.active = TabIndex(target);
                    self.new_focused = Some(path);
                    let target_id = self
                        .id
                        .with((path.surface, "surface"))
                        .with((path.node, "node"))
                        .with((target, "tab"));
                    tabs_ui.memory_mut(|m| m.request_focus(target_id));
                    roved = true;
                }
            }

            tab_viewer.on_tab_button(tab, &response);

            if self.show_close_buttons && tab_viewer.is_closeable(tab) && response.middle_clicked()
            {
                self.to_remove.push(TabRemoval::Tab(
                    (path, tab_index).into(),
                    ForcedRemoval(false),
                ));
            }
        }

        tab_hovered
    }

    /// Draws the tab add button.
    #[allow(clippy::too_many_arguments)]
    fn tab_plus(
        &mut self,
        ui: &mut Ui,
        path: NodePath,
        tab_viewer: &mut impl TabViewer<Tab = Tab>,
        tabbar_outer_rect: Rect,
        offset: f32,
        fade_style: Option<&Style>,
        bar: (Rect, CornerRadius),
    ) {
        let rect = Rect::from_min_max(
            tabbar_outer_rect.right_top() - vec2(Style::TAB_ADD_BUTTON_SIZE + offset, 0.0),
            tabbar_outer_rect.right_bottom() - vec2(offset, 2.0),
        );

        let ui = &mut ui.new_child(
            UiBuilder::new()
                .max_rect(rect)
                .layout(Layout::left_to_right(Align::Center))
                .id_salt((path.node, "tab_add")),
        );

        let (rect, mut response) = ui.allocate_exact_size(ui.available_size(), Sense::click());
        let enabled = ui.is_enabled();
        response.widget_info(|| WidgetInfo::labeled(Role::Button, enabled, &self.labels.add_tab));

        response = response.on_hover_cursor(CursorIcon::PointingHand);

        let style = fade_style.unwrap_or_else(|| self.style.as_ref().unwrap());
        let color = if response.hovered() || response.has_focus() {
            ui.painter().rect_filled(
                rect,
                corners_within(rect, bar.0, bar.1),
                style.buttons.add_tab_bg_fill,
            );
            style.buttons.add_tab_active_color
        } else {
            style.buttons.add_tab_color
        };

        let mut plus_rect = rect;

        rect_set_size_centered(&mut plus_rect, Vec2::splat(Style::TAB_ADD_PLUS_SIZE));

        ui.painter().line_segment(
            [plus_rect.center_top(), plus_rect.center_bottom()],
            Stroke::new(1.0_f32, color),
        );
        ui.painter().line_segment(
            [plus_rect.right_center(), plus_rect.left_center()],
            Stroke::new(1.0_f32, color),
        );

        // Draw button left border.
        ui.painter().vline(
            rect.left(),
            rect.y_range(),
            Stroke::new(
                ui.ctx().pixels_per_point().recip(),
                style.buttons.add_tab_border_color,
            ),
        );

        let popup_id = ui.scope_id().with("tab_add_popup");
        if self.show_add_popup {
            Popup::from_toggle_button_response(&response)
                .id(popup_id)
                .close_behavior(PopupCloseBehavior::CloseOnClickOutside)
                .show(|ui| {
                    tab_viewer.add_popup(ui, path);
                });
        }

        if response.clicked() {
            tab_viewer.on_add(path);
        }
    }

    /// Draws the close all button.
    #[allow(clippy::too_many_arguments)]
    #[allow(unused_assignments)]
    fn tab_close_all(
        &mut self,
        ui: &mut Ui,
        path: NodePath,
        tabbar_outer_rect: Rect,
        fade_style: Option<&Style>,
        disabled: bool,
        close_window_disabled: bool,
        bar: (Rect, CornerRadius),
    ) {
        let rect = Rect::from_min_max(
            tabbar_outer_rect.right_top() - vec2(Style::TAB_CLOSE_ALL_BUTTON_SIZE, 0.0),
            tabbar_outer_rect.right_bottom() - vec2(0.0, 2.0),
        );

        let ui = &mut ui.new_child(
            UiBuilder::new()
                .max_rect(rect)
                .layout(Layout::left_to_right(Align::Center))
                .id_salt((path.node, "tab_close_all")),
        );

        let (rect, mut response) = ui.allocate_exact_size(ui.available_size(), Sense::click());

        let style = fade_style.unwrap_or_else(|| self.style.as_ref().unwrap());

        // Whether we're on "secondary button mode" due to modifier keys
        let on_secondary_button = self.is_on_secondary_button(path.surface, ui, &response);

        let (label, enabled) = if on_secondary_button {
            (&self.labels.close_window, !close_window_disabled)
        } else {
            (&self.labels.close_all_tabs, !disabled)
        };
        let enabled = enabled && ui.is_enabled();
        response.widget_info(|| WidgetInfo::labeled(Role::Button, enabled, label));

        let mut stroke_color = if disabled {
            style.buttons.close_all_tabs_disabled_color
        } else if response.hovered() || response.has_focus() {
            if !(close_window_disabled && on_secondary_button) {
                ui.painter().rect_filled(
                    rect,
                    corners_within(rect, bar.0, bar.1),
                    style.buttons.close_all_tabs_bg_fill,
                );
            }
            style.buttons.close_all_tabs_active_color
        } else {
            style.buttons.close_all_tabs_color
        };

        let mut close_all_rect = rect;

        rect_set_size_centered(&mut close_all_rect, Vec2::splat(Style::TAB_CLOSE_ALL_SIZE));

        if !disabled {
            response = response.on_hover_cursor(CursorIcon::PointingHand);
        }

        if on_secondary_button {
            // Close the entire window
            if close_window_disabled {
                stroke_color = style.buttons.close_all_tabs_disabled_color;
                response = response
                    .on_hover_cursor(CursorIcon::NotAllowed)
                    .on_hover_text(
                        self.dock_state
                            .translations
                            .leaf
                            .close_all_button_disabled_tooltip
                            .as_str(),
                    );
            }
            Self::draw_close_window_symbol(ui, stroke_color, close_all_rect);
        } else {
            // Close all tabs in this leaf
            if !disabled {
                if self.dock_state.is_window(path.surface) && self.secondary_button_context_menu {
                    response.context_menu(|ui| {
                        ui.add_enabled_ui(!close_window_disabled, |ui| {
                            if ui
                                .button(&self.dock_state.translations.leaf.close_all_button)
                                .on_disabled_hover_text(
                                    self.dock_state
                                        .translations
                                        .leaf
                                        .close_all_button_disabled_tooltip
                                        .as_str(),
                                )
                                .clicked()
                            {
                                self.to_remove.push(TabRemoval::Window(path.surface));
                            }
                        });
                    });
                }
            } else {
                response = response
                    .on_hover_cursor(CursorIcon::NotAllowed)
                    .on_hover_text(
                        self.dock_state
                            .translations
                            .leaf
                            .close_button_disabled_tooltip
                            .as_str(),
                    );
            }

            if response.clicked() {
                if on_secondary_button {
                    if !close_window_disabled {
                        self.to_remove.push(TabRemoval::Window(path.surface));
                    }
                } else if !disabled {
                    self.to_remove.push(TabRemoval::Node(path));
                }
            }

            ui.painter().line_segment(
                [close_all_rect.left_top(), close_all_rect.right_bottom()],
                Stroke::new(1.0_f32, stroke_color),
            );
            ui.painter().line_segment(
                [close_all_rect.right_top(), close_all_rect.left_bottom()],
                Stroke::new(1.0_f32, stroke_color),
            );
        }

        // Draw button left border.
        ui.painter().vline(
            rect.left(),
            rect.y_range(),
            Stroke::new(
                ui.ctx().pixels_per_point().recip(),
                style.buttons.close_all_tabs_border_color,
            ),
        );

        if !disabled && !on_secondary_button {
            response = self.show_tooltip_hints(path.surface, response);
        }
    }

    /// Draws the collapse button.
    fn tab_collapse(
        &mut self,
        ui: &mut Ui,
        path: NodePath,
        tabbar_outer_rect: Rect,
        fade_style: Option<&Style>,
        collapsed: bool,
        bar: (Rect, CornerRadius),
    ) {
        let rect = Rect::from_min_max(
            tabbar_outer_rect.left_top(),
            tabbar_outer_rect.left_bottom() + vec2(Style::TAB_COLLAPSE_BUTTON_SIZE, 0.0),
        );

        let ui = &mut ui.new_child(
            UiBuilder::new()
                .max_rect(rect)
                .layout(Layout::left_to_right(Align::Center))
                .id_salt((path.node, "tab_collapse")),
        );

        let (rect, mut response) = ui.allocate_exact_size(ui.available_size(), Sense::click());

        response = response.on_hover_cursor(CursorIcon::PointingHand);

        let style = fade_style.unwrap_or_else(|| self.style.as_ref().unwrap());

        // Whether we're on "secondary button mode" due to modifier keys
        let on_secondary_button = self.is_on_secondary_button(path.surface, ui, &response);

        let enabled = ui.is_enabled();
        if on_secondary_button {
            response.widget_info(|| {
                WidgetInfo::labeled(Role::Button, enabled, &self.labels.minimize_window)
            });
        } else {
            response
                .widget_info(|| WidgetInfo::labeled(Role::Button, enabled, &self.labels.collapse));
            ui.ctx()
                .accesskit_node_builder(response.id, |node| node.set_expanded(!collapsed));
        }

        let color = if response.hovered() || response.has_focus() {
            ui.painter().rect_filled(
                rect,
                corners_within(rect, bar.0, bar.1),
                style.buttons.collapse_tabs_bg_fill,
            );
            style.buttons.collapse_tabs_active_color
        } else {
            style.buttons.collapse_tabs_color
        };

        let mut arrow_rect = rect;
        rect_set_size_centered(&mut arrow_rect, Vec2::splat(Style::TAB_COLLAPSE_ARROW_SIZE));

        if on_secondary_button {
            // Collapse the entire window
            Self::draw_chevron_down(ui, style, color, arrow_rect);
        } else {
            // Draw arrow.
            Self::draw_arrow(collapsed, ui, color, arrow_rect);
        }

        // Draw button right border.
        ui.painter().vline(
            rect.right(),
            rect.y_range(),
            Stroke::new(
                ui.ctx().pixels_per_point().recip(),
                style.buttons.collapse_tabs_border_color,
            ),
        );

        if response.clicked() {
            if on_secondary_button {
                self.window_toggle_minimized(path.surface);
            } else {
                self.dock_state[path].set_collapsed(!collapsed);
                self.dock_state[path.surface].node_update_collapsed(path.node);
                self.window_update_collapsed(path);
            }
        }

        if self.dock_state.is_window(path.surface) && self.secondary_button_context_menu {
            response.context_menu(|ui| {
                if ui
                    .button(&self.dock_state.translations.leaf.minimize_button)
                    .clicked()
                {
                    ui.close();
                    self.window_toggle_minimized(path.surface);
                }
            });
        }

        if !on_secondary_button {
            self.show_tooltip_hints(path.surface, response);
        }
    }

    fn show_tooltip_hints(&mut self, surface_index: SurfaceIndex, response: Response) -> Response {
        if self.dock_state.is_window(surface_index)
            && self.show_secondary_button_hint
            && (self.secondary_button_context_menu || self.secondary_button_on_modifier)
        {
            let hint = if self.secondary_button_context_menu && self.secondary_button_on_modifier {
                &self
                    .dock_state
                    .translations
                    .leaf
                    .minimize_button_modifier_menu_hint
            } else if self.secondary_button_context_menu {
                &self.dock_state.translations.leaf.minimize_button_menu_hint
            } else {
                &self
                    .dock_state
                    .translations
                    .leaf
                    .minimize_button_modifier_hint
            };
            return response.on_hover_text(hint);
        }
        response
    }

    fn is_on_secondary_button(
        &self,
        surface_index: SurfaceIndex,
        ui: &mut Ui,
        response: &Response,
    ) -> bool {
        self.dock_state.is_window(surface_index)
            && self.secondary_button_on_modifier
            && ui.input(|i| {
                i.modifiers
                    .matches_logically(self.secondary_button_modifiers)
            })
            && (response.hovered() || response.has_focus() || response.is_pointer_button_down_on())
    }

    fn draw_close_window_symbol(ui: &mut Ui, stroke_color: Color32, close_all_rect: Rect) {
        ui.painter().add(Shape::line(
            vec![
                close_all_rect
                    .right_center()
                    .lerp(close_all_rect.right_bottom(), 0.5),
                close_all_rect.right_bottom(),
                close_all_rect.left_bottom(),
                close_all_rect.left_top(),
                close_all_rect
                    .center_top()
                    .lerp(close_all_rect.left_top(), 0.5),
            ],
            Stroke::new(1.0_f32, stroke_color),
        ));
        ui.painter().line_segment(
            [close_all_rect.center_top(), close_all_rect.right_center()],
            Stroke::new(1.0_f32, stroke_color),
        );
        ui.painter().line_segment(
            [close_all_rect.center(), close_all_rect.right_top()],
            Stroke::new(1.0_f32, stroke_color),
        );
    }

    /// Checks if a drag has exceeded the movement threshold, and if so,
    /// stores DragData for the given tab. Returns the drag delta if stored.
    fn try_initiate_tab_drag(
        &self,
        ui: &Ui,
        state: &mut State,
        path: NodePath,
        tab_index: TabIndex,
    ) -> Option<Vec2> {
        let pointer_pos = ui.ctx().pointer_interact_pos()?;
        let start = *state.drag_start.get_or_insert(pointer_pos);
        let delta = pointer_pos - start;
        (delta.x.abs() > 30.0 || delta.y.abs() > 6.0).then(|| {
            state.dragged = Some(DraggedTab {
                path: (path, tab_index).into(),
                pointer: pointer_pos,
            });
            ui.memory_mut(|mem| {
                mem.data.insert_temp(
                    self.id.with("drag_data"),
                    Some(DragData {
                        src: TreeComponent::Tab((path, tab_index).into()),
                        rect: self.dock_state[path].rect().unwrap(),
                    }),
                );
            });
            delta
        })
    }

    fn draw_arrow(collapsed: bool, ui: &mut Ui, color: Color32, arrow_rect: Rect) {
        ui.painter().add(Shape::convex_polygon(
            if collapsed {
                // Arrow pointing rightwards.
                vec![
                    arrow_rect.left_top(),
                    arrow_rect.right_center(),
                    arrow_rect.left_bottom(),
                ]
            } else {
                // Arrow pointing downwards.
                vec![
                    arrow_rect.left_top(),
                    arrow_rect.right_top(),
                    arrow_rect.center_bottom(),
                ]
            },
            color,
            Stroke::NONE,
        ));
    }

    fn draw_chevron_down(ui: &mut Ui, style: &Style, color: Color32, arrow_rect: Rect) {
        ui.painter().add(Shape::convex_polygon(
            // Arrow pointing downwards.
            vec![
                arrow_rect.left_top(),
                arrow_rect.right_top(),
                arrow_rect.center(),
            ],
            color,
            Stroke::NONE,
        ));

        // Chevron pointing downwards.
        ui.painter().add(Shape::convex_polygon(
            vec![
                arrow_rect.left_center(),
                arrow_rect.right_center(),
                arrow_rect.center_bottom(),
            ],
            color,
            Stroke::NONE,
        ));
        let color = style.buttons.minimize_window_bg_fill;
        ui.painter().add(Shape::convex_polygon(
            vec![
                arrow_rect
                    .left_center()
                    .lerp(arrow_rect.right_center(), 0.25),
                arrow_rect
                    .left_center()
                    .lerp(arrow_rect.right_center(), 0.75),
                arrow_rect.center().lerp(arrow_rect.center_bottom(), 0.5),
            ],
            color,
            Stroke::NONE,
        ));
    }

    /// Updates the collapsed state of the node and its parents.
    fn window_update_collapsed(&mut self, path: NodePath) {
        let surface = &mut self.dock_state[path.surface];
        let collapsed = surface[path.node].is_collapsed();
        if !collapsed {
            if let Some(window_state) = self.dock_state.get_window_state_mut(path.surface) {
                window_state.set_new(true);
            }
        } else if surface.root_node().is_some_and(|root| root.is_collapsed()) {
            let root_index = NodeIndex::root();
            let surface_height = if surface.root_node().is_some() {
                surface[root_index].rect().unwrap().height()
            } else {
                0.0
            };
            if let Some(window_state) = self.dock_state.get_window_state_mut(path.surface) {
                window_state.set_expanded_height(surface_height);
            }
        }
    }

    /// Lays out, interacts with and paints one tab. `state` holds the flags known before
    /// layout: selected, dragged, and whether its leaf is focused.
    ///
    /// Returns the main button response plus the response of the close button, if any.
    #[allow(clippy::too_many_arguments)]
    fn tab_title(
        &mut self,
        ui: &mut Ui,
        tab_style: &TabStyle,
        id: Id,
        label: WidgetText,
        state: PartState,
        bar: Rect,
        preferred_width: Option<f32>,
        show_close_button: bool,
        fade: Option<&Style>,
        folded: bool,
    ) -> (Response, Option<Response>) {
        let style = fade.unwrap_or_else(|| self.style.as_ref().unwrap());
        let galley = label.into_galley(ui, None, f32::INFINITY, TextStyle::Button);
        let close_button_size = if show_close_button {
            Style::TAB_CLOSE_BUTTON_SIZE.min(style.tab_bar.height)
        } else {
            0.0
        };
        let active = state.contains(PartState::SELECTED) || state.contains(PartState::DRAGGED);

        let measured = self.painter.tab_width(&TabMeasure {
            title: &galley,
            state,
            close_width: close_button_size,
            style,
            tab_style,
        });
        let tab_width = preferred_width.unwrap_or(0.0).at_least(measured);

        let (_, tab_rect) = ui.allocate_space(vec2(tab_width, ui.available_height()));
        // With roving focus, only the active tab is a Tab stop.
        let roving = self.tab_focus == TabFocus::Roving;
        let sense = if roving && !active {
            Sense::CLICK | Sense::DRAG
        } else {
            Sense::click_and_drag()
        };
        let mut response = ui.interact(tab_rect, id, sense);
        {
            // Expose the tab to assistive technology: role `Tab`, named by its title.
            // AccessKit's `selected` is set directly, as `WidgetInfo::selected` maps to `toggled`.
            let title = galley.clone();
            let enabled = ui.is_enabled();
            response.widget_info(|| WidgetInfo::labeled(Role::Tab, enabled, title.text()));
            ui.ctx()
                .accesskit_node_builder(response.id, |node| node.set_selected(active));
        }
        if ui.ctx().dragged_id().is_none() && self.draggable_tabs {
            response = response.on_hover_cursor(CursorIcon::Grab);
        }

        let close_response = show_close_button.then(|| {
            let mut text_rect = tab_rect;
            text_rect.set_width(text_rect.width() - close_button_size);
            let mut close_button_rect = tab_rect;
            close_button_rect.set_left(text_rect.right());
            close_button_rect =
                Rect::from_center_size(close_button_rect.center(), Vec2::splat(close_button_size));

            let close_response = ui
                .interact(
                    close_button_rect,
                    id.with("close-button"),
                    if roving { Sense::CLICK } else { Sense::click() },
                )
                .on_hover_cursor(CursorIcon::PointingHand);
            let enabled = ui.is_enabled();
            close_response
                .widget_info(|| WidgetInfo::labeled(Role::Button, enabled, &self.labels.close_tab));
            close_response
        });

        let state = state
            .with(PartState::HOVERED, response.hovered())
            .with(PartState::PRESSED, response.is_pointer_button_down_on())
            .with(PartState::FOCUSED, response.has_focus());
        let close = close_response.as_ref().map(|close| TabClose {
            rect: close.rect,
            state: PartState::NONE
                .with(PartState::HOVERED, close.hovered())
                .with(PartState::PRESSED, close.is_pointer_button_down_on())
                .with(PartState::FOCUSED, close.has_focus()),
        });
        self.painter.tab(
            ui.painter(),
            &TabPaint {
                rect: tab_rect,
                bar,
                title: &galley,
                state,
                close,
                folded,
                style,
                tab_style,
            },
        );

        (response, close_response)
    }

    #[allow(clippy::too_many_arguments)]
    fn tab_bar_scroll(
        &mut self,
        ui: &mut Ui,
        state: &State,
        path: NodePath,
        actual_width: f32,
        available_width: f32,
        scroll_bar_width: f32,
        tabbar_response: &Response,
        tab_hovered: bool,
        fade_style: Option<&Style>,
    ) {
        if available_width <= 0.0 {
            return;
        }

        let leaf = self.dock_state[path]
            .get_leaf_mut()
            .expect("This node must be a leaf");
        let overflow = (actual_width - available_width).at_least(0.0);
        let style = fade_style.unwrap_or_else(|| self.style.as_ref().unwrap());

        // Compare to 1.0 and not 0.0 to avoid drawing a scroll bar due
        // to floating point precision issue during tab drawing.
        if overflow > 1.0 {
            if style.tab_bar.show_scroll_bar_on_overflow {
                // Draw scroll bar
                let bar_height = 7.5;
                let (scroll_bar_rect, _scroll_bar_response) = ui.allocate_exact_size(
                    vec2(scroll_bar_width, bar_height),
                    Sense::click_and_drag(),
                );

                // Compute scroll bar handle position and size.
                let overflow_ratio = actual_width / available_width;
                let scroll_ratio = -leaf.scroll / overflow;

                let scroll_bar_handle_size = overflow_ratio.recip() * scroll_bar_rect.width();
                let scroll_bar_handle_start = lerp(
                    scroll_bar_rect.left()..=scroll_bar_rect.right() - scroll_bar_handle_size,
                    scroll_ratio,
                );
                let scroll_bar_handle_rect = Rect::from_min_size(
                    pos2(scroll_bar_handle_start, scroll_bar_rect.min.y),
                    vec2(scroll_bar_handle_size, bar_height),
                );

                let scroll_bar_handle_response = ui.interact(
                    scroll_bar_handle_rect,
                    self.id.with((path.node, "node")),
                    Sense::drag(),
                );

                // Coefficient to apply to input displacements so that we move the scroll by the correct amount.
                let points_to_scroll_coefficient =
                    overflow / (scroll_bar_rect.width() - scroll_bar_handle_size);

                leaf.scroll -=
                    scroll_bar_handle_response.drag_delta().x * points_to_scroll_coefficient;

                if let Some(pos) = state.last_hover_pos
                    && scroll_bar_rect.contains(pos)
                {
                    leaf.scroll += ui.input(|i| i.smooth_scroll_delta.y + i.smooth_scroll_delta.x)
                        * points_to_scroll_coefficient;
                }

                // Draw the bar.
                ui.painter()
                    .rect_filled(scroll_bar_rect, 0.0, ui.visuals().extreme_bg_color);

                ui.painter().rect_filled(
                    scroll_bar_handle_rect,
                    bar_height / 2.0,
                    ui.visuals()
                        .widgets
                        .style(&scroll_bar_handle_response)
                        .bg_fill,
                );
            }

            // Handle user input.
            if tabbar_response.hovered() || tab_hovered {
                leaf.scroll += ui.input(|i| i.smooth_scroll_delta.y + i.smooth_scroll_delta.x);
            }
        }

        leaf.scroll = leaf.scroll.clamp(-overflow, 0.0);
    }

    #[allow(clippy::too_many_arguments)]
    fn tab_body(
        &mut self,
        ui: &mut Ui,
        state: &State,
        path: NodePath,
        tab_viewer: &mut impl TabViewer<Tab = Tab>,
        spacing: Vec2,
        tabbar_rect: Rect,
        fade: Option<(&Style, f32)>,
        collapsed: bool,
    ) {
        let (body_rect, body_response) =
            ui.allocate_exact_size(ui.available_size_before_wrap(), Sense::hover());

        let focused = self.dock_state.focused_leaf() == Some(path);
        let leaf = self
            .dock_state
            .leaf_mut(path)
            .expect("This node must be a leaf");
        let tab_bar_shown = !(self.hidable_tab_bars && leaf.tab_bar_hidden);
        let LeafNode {
            rect,
            viewport,
            tabs,
            active,
            ..
        } = leaf;
        if !collapsed && let Some(tab) = tabs.get_mut(active.0) {
            if *viewport != body_rect {
                *viewport = body_rect;
                tab_viewer.on_rect_changed(tab);
            }

            if ui.input(|i| i.pointer.any_click())
                && let Some(pos) = state.last_hover_pos
                && body_rect.contains(pos)
                && !tabbar_rect.contains(pos)
                && Some(ui.layer_id()) == ui.ctx().layer_id_at(pos)
            {
                self.new_focused = Some(path);
            }

            let (style, fade_factor) = fade.unwrap_or_else(|| (self.style.as_ref().unwrap(), 1.0));
            let tabs_styles = tab_viewer.tab_style_override(tab, &style.tab);

            let tabs_style = tabs_styles.as_ref().unwrap_or(&style.tab);
            // Under its tab bar, the body's top corners are square: they join the bar.
            let body_radius = if tab_bar_shown {
                Attached::Above.corner_radius(tabs_style.tab_body.corner_radius)
            } else {
                tabs_style.tab_body.corner_radius
            };

            if tab_viewer.clear_background(tab) {
                ui.painter()
                    .rect_filled(body_rect, body_radius, tabs_style.tab_body.bg_fill);
            }

            // Construct a new ui with the correct tab id.
            //
            // We are forced to use `Ui::new` because other methods (eg: push_id) always mix
            // the provided id with their own which would cause tabs to change id when moved
            // from node to node.
            let id = tab_body_id(self.id, path, tab_viewer.id(tab));
            ui.ctx().check_for_id_clash(id, body_rect, "a tab with id");
            let ui = &mut Ui::new(
                ui.ctx().clone(),
                id,
                UiBuilder::new().max_rect(body_rect).layer_id(ui.layer_id()),
            );
            ui.set_clip_rect(Rect::from_min_max(ui.cursor().min, ui.clip_rect().max));

            // Use initial spacing for ui.
            ui.spacing_mut().item_spacing = spacing;

            let body_state = PartState::NONE
                .with(PartState::HOVERED, body_response.contains_pointer())
                .with(PartState::FOCUSED, focused)
                .with(PartState::LEAF_FOCUSED, focused);
            self.painter.tab_body(
                ui.painter(),
                &TabBodyPaint {
                    rect: ui.clip_rect(),
                    corner_radius: body_radius,
                    state: body_state,
                    style,
                    tab_style: tabs_style,
                },
            );

            ScrollArea::new(tab_viewer.scroll_bars(tab)).show(ui, |ui| {
                Frame::new()
                    .inner_margin(tabs_style.tab_body.inner_margin)
                    .show(ui, |ui| {
                        if fade_factor != 1.0 {
                            fade_visuals(ui.visuals_mut(), fade_factor);
                        }
                        let available_rect = ui.available_rect_before_wrap();
                        ui.expand_to_include_rect(available_rect);
                        tab_viewer.ui(ui, tab);
                    });
            });
        }

        // change hover destination
        if let Some(pointer) = state.last_hover_pos {
            // Prevent borrow checker issues.
            let rect = rect.to_owned();

            // if the dragged tab isn't allowed in a window,
            // it's unnecessary to change the hover state
            let is_dragged_valid = match &state.dnd {
                Some(DragDropState {
                    drag: DragData { src, .. },
                    ..
                }) => match *src {
                    TreeComponent::Tab(src_path) => {
                        !self.dock_state.is_window(path.surface)
                            || !self.dock_state[src_path.node_path()].is_leaf()
                            || self.may_float(src_path, tab_viewer)
                    }
                    _ => unreachable!("collections of nodes can't be dragged (yet)"),
                },
                _ => true,
            };

            // Use rect.contains instead of response.hovered as the dragged tab covers
            // the underlying responses.
            if state.drag_start.is_some()
                && rect.contains(pointer)
                && is_dragged_valid
                && !self.covered_by_window(ui, pointer)
            {
                let on_title_bar = tabbar_rect.contains(pointer);
                let (dst, tab) = {
                    match self.tab_hover_rect {
                        Some((rect, tab_index)) => {
                            (TreeComponent::Tab((path, tab_index).into()), Some(rect))
                        }
                        None => (
                            TreeComponent::Node(path),
                            on_title_bar.then_some(tabbar_rect),
                        ),
                    }
                };

                ui.memory_mut(|mem| {
                    mem.data.insert_temp(
                        self.id.with("hover_data"),
                        Some(HoverData {
                            rect,
                            dst,
                            tab,
                            outside: false,
                        }),
                    );
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::tab_body_id;
    use crate::{NodeIndex, NodePath, SurfaceIndex};
    use egui::Id;

    #[test]
    fn tab_body_ids_differ_between_surfaces() {
        let dock_area_id = Id::unique("dock-area");
        let tab_id = Id::unique("same-tab");
        let node = NodeIndex::root();

        let main = NodePath {
            surface: SurfaceIndex::main(),
            node,
        };
        let detached = NodePath {
            surface: SurfaceIndex(1),
            node,
        };

        assert_ne!(
            tab_body_id(dock_area_id, main, tab_id),
            tab_body_id(dock_area_id, detached, tab_id)
        );
    }
}
