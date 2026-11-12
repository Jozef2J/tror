use egui::{Rect, pos2, vec2};

use super::*;

fn rect(width: f32) -> Rect {
    Rect::from_min_size(pos2(0.0, 0.0), vec2(width, 100.0))
}

fn width(laid: &Laid, name: &str) -> f32 {
    laid.rect(name).unwrap().width()
}

fn side(name: &str, default: f32, min: f32) -> Zone {
    Zone::leaf(name, Content::Dock(name.into())).with(|c| {
        c.default = Some(Size::Points(default));
        c.min = Some(Size::Points(min));
    })
}

fn centre(min: f32) -> Zone {
    Zone::leaf("centre", Content::Dock("centre".into())).with(|c| {
        c.grow = true;
        c.min = Some(Size::Points(min));
    })
}

fn row(children: Vec<Zone>) -> Layout {
    Layout::new(Zone::split("root", Dir::Horizontal, children))
}

/// The usual shell: two sides around a centre.
fn shell() -> Layout {
    row(vec![
        side("left", 200.0, 50.0),
        centre(300.0),
        side("right", 200.0, 50.0),
    ])
}

#[test]
fn the_sides_get_their_wanted_size_and_the_centre_the_rest() {
    let laid = shell().solve(rect(1000.0), &Solve::default());
    assert_eq!(width(&laid, "left"), 200.0);
    assert_eq!(width(&laid, "centre"), 600.0);
    assert_eq!(width(&laid, "right"), 200.0);
    assert_eq!(laid.rect("right").unwrap().min.x, 800.0);
}

#[test]
fn the_centre_wins_over_the_sides_wanted_size_and_max() {
    let mut layout = shell();
    layout.constrain("left", |c| c.max = Some(Size::Points(400.0)));
    layout.zone_mut("left").unwrap().state.size = Some(400.0);
    let laid = layout.solve(rect(600.0), &Solve::default());
    assert_eq!(width(&laid, "centre"), 300.0);
    // Of two sides as near and of the same priority, the first gives first;
    // both stay at or over their least size.
    assert!(width(&laid, "left") >= 50.0 && width(&laid, "right") >= 50.0);
    assert_eq!(width(&laid, "left") + width(&laid, "right"), 300.0);
    // The user's size is kept for when the room comes back.
    assert_eq!(layout.zone("left").unwrap().state.size, Some(400.0));
}

#[test]
fn the_lowest_priority_gives_way_first() {
    let mut layout = shell();
    layout.constrain("left", |c| c.priority = Priority::High);
    layout.constrain("right", |c| c.priority = Priority::Low);
    let laid = layout.solve(rect(600.0), &Solve::default());
    // 100 short: the right side gives it all.
    assert_eq!(width(&laid, "left"), 200.0);
    assert_eq!(width(&laid, "right"), 100.0);
    assert_eq!(width(&laid, "centre"), 300.0);
    // 250 short: the right side down to its least, then the left.
    let laid = layout.solve(rect(450.0), &Solve::default());
    assert_eq!(width(&laid, "right"), 50.0);
    assert_eq!(width(&laid, "left"), 100.0);
}

#[test]
fn the_nearest_to_the_centre_gives_way_first() {
    let layout = row(vec![
        side("far", 100.0, 20.0),
        side("near", 100.0, 20.0),
        centre(300.0),
    ]);
    let laid = layout.solve(rect(450.0), &Solve::default());
    assert_eq!(width(&laid, "near"), 50.0);
    assert_eq!(width(&laid, "far"), 100.0);
    let laid = layout.solve(rect(360.0), &Solve::default());
    assert_eq!(width(&laid, "near"), 20.0);
    assert_eq!(width(&laid, "far"), 40.0);
}

#[test]
fn least_sizes_are_never_violated() {
    let layout = shell();
    for room in [1000.0, 600.0, 450.0, 400.0] {
        let laid = layout.solve(rect(room), &Solve::default());
        assert!(width(&laid, "left") >= 50.0, "room {room}");
        assert!(width(&laid, "right") >= 50.0, "room {room}");
        assert!(width(&laid, "centre") >= 300.0, "room {room}");
    }
    // Short even at the least sizes: each zone is given its least size and
    // the layout is cut at the edge.
    let laid = layout.solve(rect(380.0), &Solve::default());
    assert_eq!(width(&laid, "left"), 50.0);
    assert_eq!(width(&laid, "centre"), 300.0);
    assert_eq!(width(&laid, "right"), 30.0);
}

#[test]
fn fit_content_raises_the_least_size() {
    let mut layout = shell();
    layout.constrain("right", |c| c.fit_content = true);
    let mut solve = Solve::default();
    solve.content.insert("right".into(), 150.0);
    let laid = layout.solve(rect(500.0), &solve);
    assert_eq!(width(&laid, "right"), 150.0);
    assert_eq!(width(&laid, "left"), 50.0);
}

#[test]
fn share_sizes_follow_the_parent() {
    let layout = row(vec![
        Zone::leaf("left", Content::App).with(|c| {
            c.default = Some(Size::Share(0.25));
            c.min = Some(Size::Share(0.1));
        }),
        Zone::leaf("centre", Content::App).with(|c| c.grow = true),
    ]);
    let laid = layout.solve(rect(800.0), &Solve::default());
    assert_eq!(width(&laid, "left"), 200.0);
    assert_eq!(width(&laid, "centre"), 600.0);
    let laid = layout.solve(rect(400.0), &Solve::default());
    assert_eq!(width(&laid, "left"), 100.0);
}

#[test]
fn a_split_of_equals_shares_its_length() {
    let layout = row(vec![
        Zone::leaf("a", Content::Empty),
        Zone::leaf("b", Content::Empty),
        Zone::leaf("c", Content::Empty),
        Zone::leaf("d", Content::Empty),
    ]);
    let laid = layout.solve(rect(400.0), &Solve::default());
    for name in ["a", "b", "c", "d"] {
        assert_eq!(width(&laid, name), 100.0);
    }
}

#[test]
fn nested_splits_are_solved_in_their_parent_rect() {
    let layout = Layout::new(Zone::split(
        "root",
        Dir::Horizontal,
        vec![
            side("left", 200.0, 50.0),
            Zone::split(
                "middle",
                Dir::Vertical,
                vec![
                    centre(50.0),
                    Zone::leaf("bottom", Content::App)
                        .with(|c| c.default = Some(Size::Share(0.25))),
                ],
            )
            .with(|c| c.grow = true),
        ],
    ));
    let laid = layout.solve(rect(1000.0), &Solve::default());
    let bottom = laid.rect("bottom").unwrap();
    assert_eq!(
        bottom,
        Rect::from_min_max(pos2(200.0, 75.0), pos2(1000.0, 100.0))
    );
    assert_eq!(laid.rect("centre").unwrap().height(), 75.0);
}

#[test]
fn folded_zones_take_their_strip_and_keep_their_size() {
    let mut layout = shell();
    layout.zone_mut("left").unwrap().state.folded = true;
    layout.zone_mut("left").unwrap().state.size = Some(250.0);
    let solve = Solve::default();
    let laid = layout.solve(rect(1000.0), &solve);
    let left = laid.get("left").unwrap();
    assert_eq!(left.shown, Shown::Folded);
    assert_eq!(left.strip, Some(left.rect));
    assert_eq!(left.rect.width(), solve.strip);
    assert_eq!(width(&laid, "centre"), 1000.0 - solve.strip - 200.0);
    // Unfolded, it gets the size the user gave back.
    layout.zone_mut("left").unwrap().state.folded = false;
    assert_eq!(width(&layout.solve(rect(1000.0), &solve), "left"), 250.0);
}

#[test]
fn a_zone_lying_over_keeps_its_strip_in_the_flow() {
    let mut layout = shell();
    layout.zone_mut("right").unwrap().state.over = true;
    let laid = layout.solve(rect(1000.0), &Solve::default());
    let right = laid.get("right").unwrap();
    assert_eq!(right.shown, Shown::Over);
    let strip = right.strip.unwrap();
    assert_eq!(strip.min.x, 976.0);
    // Its body opens towards the centre, over it.
    assert_eq!(
        right.rect,
        Rect::from_min_max(pos2(776.0, 0.0), pos2(976.0, 100.0))
    );
    assert_eq!(width(&laid, "centre"), 776.0);
}

#[test]
fn hidden_zones_take_no_room() {
    let mut layout = shell();
    layout.zone_mut("left").unwrap().state.hidden = true;
    let laid = layout.solve(rect(1000.0), &Solve::default());
    assert_eq!(laid.get("left").unwrap().shown, Shown::Hidden);
    assert_eq!(width(&laid, "left"), 0.0);
    assert_eq!(width(&laid, "centre"), 800.0);
}

#[test]
fn zones_fold_by_themselves_only_when_they_opt_in() {
    let mut layout = shell();
    for name in ["left", "right"] {
        layout.constrain(name, |c| c.fold = Fold::ToStrip);
    }
    layout.constrain("right", |c| c.priority = Priority::Low);
    let solve = Solve::default();
    // Short even at the least sizes (400), but nobody opted in: cut.
    let laid = layout.solve(rect(380.0), &solve);
    assert_eq!(laid.get("right").unwrap().shown, Shown::Open);
    assert_eq!(width(&laid, "centre"), 300.0);

    for name in ["left", "right"] {
        layout.constrain(name, |c| c.auto_fold = true);
    }
    let laid = layout.solve(rect(380.0), &solve);
    // The lowest priority folds first; the left side gets the room back.
    assert_eq!(laid.get("right").unwrap().shown, Shown::AutoFolded);
    assert_eq!(width(&laid, "right"), solve.strip);
    assert_eq!(laid.get("left").unwrap().shown, Shown::Open);
    assert_eq!(width(&laid, "left"), 380.0 - 300.0 - solve.strip);
    // The state is not touched: with the room back, it comes back.
    assert!(!layout.zone("right").unwrap().state.folded);
    let laid = layout.solve(rect(1000.0), &solve);
    assert_eq!(laid.get("right").unwrap().shown, Shown::Open);
    // Shorter still: both fold.
    let laid = layout.solve(rect(330.0), &solve);
    assert_eq!(laid.get("left").unwrap().shown, Shown::AutoFolded);
}

#[test]
fn the_growing_zone_never_folds() {
    let mut layout = shell();
    layout.zone_mut("centre").unwrap().state.folded = true;
    let laid = layout.solve(rect(1000.0), &Solve::default());
    assert_eq!(laid.get("centre").unwrap().shown, Shown::Open);
    assert_eq!(width(&laid, "centre"), 600.0);
}

fn shell_with_bottom() -> Layout {
    Layout::new(Zone::split(
        "root",
        Dir::Horizontal,
        vec![
            side("left", 200.0, 50.0),
            Zone::split(
                "middle",
                Dir::Vertical,
                vec![centre(100.0), side("bottom", 100.0, 40.0)],
            )
            .with(|c| c.grow = true),
            side("right", 200.0, 50.0),
        ],
    ))
}

#[test]
fn reconcile_takes_the_constraints_from_code() {
    let code = shell();
    let mut loaded = shell();
    loaded.constrain("centre", |c| c.min = None);
    loaded.zone_mut("left").unwrap().state.size = Some(321.0);
    let repair = loaded.reconcile(&code);
    assert!(repair.is_empty());
    assert_eq!(
        loaded.zone("centre").unwrap().constraints,
        code.zone("centre").unwrap().constraints
    );
    assert_eq!(loaded.zone("left").unwrap().state.size, Some(321.0));
}

#[test]
fn reconcile_adds_the_zones_code_added() {
    let mut loaded = shell();
    loaded.zone_mut("right").unwrap().state.folded = true;
    let code = shell_with_bottom();
    let repair = loaded.reconcile(&code);
    assert_eq!(
        repair.added,
        vec![ZoneName::from("middle"), ZoneName::from("bottom")]
    );
    // "centre" moved under "middle" in the code but stays where it was
    // saved: the loaded shape wins; "middle" comes after "left".
    assert_eq!(
        loaded
            .root
            .children()
            .iter()
            .map(|z| z.name.0.as_str())
            .collect::<Vec<_>>(),
        ["left", "middle", "centre", "right"]
    );
    assert_eq!(loaded.zone("middle").unwrap().children().len(), 1);
    assert!(loaded.zone("right").unwrap().state.folded);
    assert_eq!(loaded.names().len(), code.names().len());
}

#[test]
fn reconcile_drops_the_zones_code_removed() {
    let mut loaded = shell_with_bottom();
    loaded.zone_mut("left").unwrap().state.size = Some(123.0);
    let code = row(vec![side("left", 200.0, 50.0), centre(300.0)]);
    let repair = loaded.reconcile(&code);
    assert_eq!(
        repair.removed,
        vec![
            ZoneName::from("middle"),
            ZoneName::from("bottom"),
            ZoneName::from("right")
        ]
    );
    // "centre" was under "middle": it takes its place, with its state.
    assert!(repair.added.is_empty());
    assert_eq!(loaded.names(), code.names());
    assert_eq!(loaded.zone("left").unwrap().state.size, Some(123.0));
}

#[test]
fn reconcile_follows_a_leaf_turned_split() {
    let mut loaded = row(vec![side("left", 200.0, 50.0), centre(300.0)]);
    loaded.zone_mut("left").unwrap().state.size = Some(222.0);
    let code = row(vec![
        Zone::split(
            "left",
            Dir::Vertical,
            vec![
                Zone::leaf("top", Content::App),
                Zone::leaf("low", Content::App),
            ],
        ),
        centre(300.0),
    ]);
    let repair = loaded.reconcile(&code);
    assert_eq!(
        repair.added,
        vec![ZoneName::from("top"), ZoneName::from("low")]
    );
    assert_eq!(loaded.zone("left").unwrap().children().len(), 2);
    assert_eq!(loaded.zone("left").unwrap().state.size, Some(222.0));
}

#[test]
fn reconcile_replaces_a_layout_of_another_root() {
    let mut loaded = Layout::single("main");
    let code = shell();
    let repair = loaded.reconcile(&code);
    assert_eq!(loaded, code);
    assert_eq!(repair.removed, vec![ZoneName::from("main")]);
}

#[cfg(feature = "serde")]
mod serde {
    use super::*;

    #[test]
    fn a_round_trip_keeps_the_tree_and_the_state_not_the_constraints() {
        let mut layout = shell_with_bottom();
        layout.zone_mut("left").unwrap().state.size = Some(260.0);
        layout.zone_mut("bottom").unwrap().state.folded = true;
        layout.zone_mut("right").unwrap().state.over = true;
        let json = serde_json::to_string(&layout).unwrap();
        assert!(json.contains("\"version\":1"), "{json}");
        assert!(!json.contains("priority"), "{json}");
        let mut loaded: Layout = serde_json::from_str(&json).unwrap();
        assert_eq!(loaded.names(), layout.names());
        assert_eq!(
            loaded.zone("centre").unwrap().constraints,
            Constraints::default()
        );
        assert!(loaded.reconcile(&shell_with_bottom()).is_empty());
        assert_eq!(loaded, layout);
    }

    #[test]
    fn a_later_version_does_not_load() {
        let json = serde_json::to_string(&Layout::single("main"))
            .unwrap()
            .replace("\"version\":1", "\"version\":99");
        let error = serde_json::from_str::<Layout>(&json).unwrap_err();
        assert!(error.to_string().contains("99"), "{error}");
    }
}
