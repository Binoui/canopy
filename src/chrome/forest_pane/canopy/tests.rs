use super::*;
use crate::{
    browser::{Browser, TreeId, VisitId},
    forest::ForestProjection,
};
fn fixture(n: usize, two_sites: bool, waiting: bool) -> (ChromeFrame, Vec<VisitId>) {
    fixture_with_parents(
        &(0..n).map(|i| i.checked_sub(1)).collect::<Vec<_>>(),
        two_sites,
        waiting,
    )
}
fn fixture_with_parents(
    parents: &[Option<usize>],
    two_sites: bool,
    waiting: bool,
) -> (ChromeFrame, Vec<VisitId>) {
    let n = parents.len();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let mut b = Browser::open(&path).unwrap();
    let root = b
        .add_roots(&[url::Url::parse("https://hub.test/").unwrap()])
        .unwrap()[0];
    let db = rusqlite::Connection::open(&path).unwrap();
    let mut ids = vec![root];
    for (i, parent) in parents.iter().enumerate().skip(1) {
        let id: VisitId = ulid::Ulid::new().to_string().parse().unwrap();
        db.execute(
            "INSERT INTO visits(id,parent_id,url,title,created_at,opened_at) VALUES(?,?,?,?,?,?)",
            rusqlite::params![
                id.to_string(),
                parent.map(|p| ids[p].to_string()),
                format!(
                    "https://{}/{i}",
                    if two_sites { "test.test" } else { "hub.test" }
                ),
                format!("Question {i}"),
                i as i64,
                (!waiting).then_some(i as i64)
            ],
        )
        .unwrap();
        ids.push(id);
    }
    if !waiting {
        for id in ids.iter().skip(1) {
            db.execute(
                "INSERT INTO visit_events(visit_id,kind,at) VALUES(?,'new',1)",
                [id.to_string()],
            )
            .unwrap();
        }
    }
    for _ in 0..12 {
        db.execute(
            "INSERT INTO visit_events(visit_id,kind,at) VALUES(?,'return',1)",
            [root.to_string()],
        )
        .unwrap();
    }
    let current = if waiting { root } else { ids[n - 1] };
    db.execute("UPDATE session SET current_id=?", [current.to_string()])
        .unwrap();
    db.execute(
        "UPDATE tree_state SET last_visit_id=?",
        [current.to_string()],
    )
    .unwrap();
    let forest =
        ForestProjection::from_snapshot(b.snapshot().unwrap(), b.tree_states().unwrap()).unwrap();
    let mut frame = super::super::tests::fixture(1, 1);
    frame.forest = forest;
    frame.shown = b.showings().unwrap();
    (frame, ids)
}
#[test]
fn fold_owner_hub_and_six_page_chain_at_every_level() {
    let (f, ids) = fixture(7, true, false);
    let facts = Facts {
        forest: &f.forest,
        previous: Some(ids[0]),
        live: &f.live,
        playing: &f.playing,
        shown: &f.shown,
    };
    let tree = &f.forest.tabs()[0];
    let far = items::fold(&facts, tree, Level::Far);
    assert_eq!(
        far.iter().map(|i| i.pages.len()).collect::<Vec<_>>(),
        [1, 6]
    );
    assert_eq!(far[1].go, ids[6]);
    assert!(far[0].appearance.previous);
    assert!(far[1].appearance.current);
    let mid = items::fold(&facts, tree, Level::Middle);
    assert_eq!(
        mid.iter().map(|i| i.pages.len()).collect::<Vec<_>>(),
        [1, 5, 1]
    );
    assert_eq!(
        mid.iter()
            .filter(|i| i.named)
            .map(|i| i.go)
            .collect::<Vec<_>>(),
        [ids[0], ids[6]]
    );
    assert_eq!(mid[1].go, ids[5]);
    let near = items::fold(&facts, tree, Level::Near);
    assert_eq!(near.iter().map(|i| i.go).collect::<Vec<_>>(), ids);
    for l in Level::ALL {
        for i in items::fold(&facts, tree, l) {
            let v = i.paint(&facts, l);
            assert!(v["tip"]
                .as_str()
                .unwrap()
                .contains(items::site(&f.forest, i.go)));
        }
    }
}
#[test]
fn ninety_page_run_preserves_both_roles_and_folds_at_each_level() {
    let (f, ids) = fixture(90, false, false);
    let facts = Facts {
        forest: &f.forest,
        previous: Some(ids[0]),
        live: &f.live,
        playing: &f.playing,
        shown: &f.shown,
    };
    let tree = &f.forest.tabs()[0];
    let far = items::fold(&facts, tree, Level::Far);
    assert_eq!(far.len(), 1);
    assert_eq!(far[0].pages.len(), 90);
    assert!(far[0].appearance.current && far[0].appearance.previous);
    assert_eq!(far[0].go, ids[0]);
    assert_eq!(
        far[0].paint(&facts, Level::Far)["anchorPage"],
        ids[89].to_string()
    );
    assert_eq!(
        far[0].paint(&facts, Level::Far)["tags"],
        json!(["You are here"])
    );
    assert_eq!(items::fold(&facts, tree, Level::Middle).len(), 3);
    assert_eq!(items::fold(&facts, tree, Level::Near).len(), 90);
}
#[test]
fn waiting_runs_choose_the_head_even_when_background_loaded() {
    let (mut f, ids) = fixture(12, false, true);
    f.loaded.extend(ids.iter().skip(1));
    let facts = Facts {
        forest: &f.forest,
        previous: None,
        live: &f.live,
        playing: &f.playing,
        shown: &f.shown,
    };
    let mut tree = f.forest.tabs()[0].clone();
    tree.id = TreeId(ids[1]);
    for l in Level::ALL {
        let folded = items::fold(&facts, &tree, l);
        assert!(!folded.is_empty());
        for i in folded.iter().filter(|i| !i.pages.contains(&ids[0])) {
            assert_eq!(i.appearance.memory, items::Memory::Waiting);
            assert_eq!(i.go, i.pages[0]);
        }
    }
}
#[test]
fn size_grows_slowly_and_is_capped_and_tips_report_showings() {
    assert_eq!(items::size(0, 20.), 20.);
    assert_eq!(items::size(1, 20.), 20.);
    assert!(items::size(3, 20.) > 20.);
    assert!(items::size(5, 20.) < 40.);
    assert_eq!(items::size(u32::MAX, 20.), 42.);
    let (f, ids) = fixture(7, true, false);
    let facts = Facts {
        forest: &f.forest,
        previous: Some(ids[0]),
        live: &f.live,
        playing: &f.playing,
        shown: &f.shown,
    };
    let i = items::fold(&facts, &f.forest.tabs()[0], Level::Middle);
    assert!(i[0].paint(&facts, Level::Middle)["tip"]
        .as_str()
        .unwrap()
        .contains("shown 12 times"));
    assert!(!i[2].paint(&facts, Level::Middle)["tip"]
        .as_str()
        .unwrap()
        .contains("shown"));
}
#[test]
fn flow_item_boxes_do_not_overlap_and_trees_fit_resting_room() {
    let chain = |n| (0..n).map(|i: usize| i.checked_sub(1)).collect::<Vec<_>>();
    let mut trail = vec![];
    let mut previous = None;
    for i in 0..34 {
        let current = trail.len();
        trail.push(previous);
        previous = Some(current);
        if i % 5 == 4 && i < 30 {
            trail.push(Some(current));
            trail.push(Some(current + 1));
        }
    }
    assert_eq!(trail.len(), 46);
    let cases = [
        (chain(7), true),
        (chain(90), false),
        (chain(500), false),
        ((0..500).map(|i| (i > 0).then_some(0)).collect(), false),
        ((0..21).map(|i| (i > 0).then_some(0)).collect(), true),
        (
            (0..39)
                .map(|i| match i {
                    0 => None,
                    1..=6 => Some(0),
                    _ => Some(i - 1),
                })
                .collect(),
            true,
        ),
        (trail, true),
    ];
    for (parents, two_sites) in cases {
        let n = parents.len();
        for level in Level::ALL {
            let (f, ids) = fixture_with_parents(&parents, two_sites, false);
            let facts = Facts {
                forest: &f.forest,
                previous: Some(ids[0]),
                live: &f.live,
                playing: &f.playing,
                shown: &f.shown,
            };
            let folded = vec![items::fold(&facts, &f.forest.tabs()[0], level)];
            for width in [390., 728., 1440., 1920.] {
                let placed = Layout::Flow.place(
                    level,
                    &folded,
                    Room {
                        width,
                        height: 800.,
                    },
                );
                let scale = level.band().min / level.band().rest;
                for (i, a) in placed.items[0].iter().enumerate() {
                    for (j, b) in placed.items[0].iter().enumerate().skip(i + 1) {
                        assert!(
                            (a.x - b.x).abs() * scale
                                >= (folded[0][i].width(level) + folded[0][j].width(level)) / 2.
                                || a.y * scale + folded[0][i].extent(level).1
                                    <= b.y * scale - folded[0][j].extent(level).0
                                || b.y * scale + folded[0][j].extent(level).1
                                    <= a.y * scale - folded[0][i].extent(level).0,
                            "overlapping boxes {n} {level:?}"
                        );
                    }
                }
                for (i, item) in folded[0].iter().enumerate() {
                    if let Some(p) = item.parent {
                        assert!(placed.items[0][i].x > placed.items[0][p].x);
                    }
                }
                let home = placed.items[0].last().unwrap();
                let pan = place::camera(
                    &placed,
                    Some((0, folded[0].len() - 1)),
                    Room {
                        width,
                        height: 800.,
                    },
                    1.,
                );
                assert!((0. ..width).contains(&(home.x + pan[0])));
            }
        }
    }
    let (f, ids) = fixture(7, true, false);
    let facts = Facts {
        forest: &f.forest,
        previous: Some(ids[0]),
        live: &f.live,
        playing: &f.playing,
        shown: &f.shown,
    };
    for l in Level::ALL {
        let trees = (0..4)
            .map(|_| items::fold(&facts, &f.forest.tabs()[0], l))
            .collect::<Vec<_>>();
        let room = Room {
            width: 1920.,
            height: 800.,
        };
        let p = Layout::Flow.place(l, &trees, room);
        if l == Level::Near {
            assert!(p.trees[1].y > p.trees[0].y);
        } else {
            assert!(p.trees[1].x > p.trees[0].x);
        }
        if l != Level::Near {
            for (t, items) in p.items.iter().zip(&trees) {
                assert!(
                    t.iter()
                        .zip(items)
                        .map(|(v, item)| v.x + item.width(l) / 2.)
                        .fold(0_f64, f64::max)
                        <= room.width
                );
            }
        }
    }
}
#[test]
fn neighbours_choose_directional_nearest_and_include_tree_names() {
    let p = [
        Point { x: 0., y: 0. },
        Point { x: 30., y: 0. },
        Point { x: 0., y: 20. },
        Point { x: 10., y: 1. },
    ];
    let nav = place::neighbours(&p);
    assert_eq!(nav[0][1], Some(3));
    assert_eq!(nav[0][3], Some(2));
    assert_eq!(nav[0][0], None);
    assert_eq!(nav[1][0], Some(3));
}
#[test]
fn value_carries_targets_menus_and_correspondence_at_every_level() {
    let (f, ids) = fixture(7, true, false);
    let mut state = State::default();
    for (request, level) in Level::ALL.into_iter().enumerate() {
        state.input(
            Input::Zoom {
                request: request as u64 + 1,
                level,
                anchor: Some(ids[5]),
            },
            &f,
        );
        let v = state.value(
            &f,
            Room {
                width: 1196.,
                height: 600.,
            },
        );
        let i = v["place"]["anchor"].as_u64().unwrap() as usize;
        assert!(v["items"][i]["pages"]
            .as_array()
            .unwrap()
            .contains(&json!(ids[5].to_string())));
        assert!(v["items"][0]["menu"]
            .as_array()
            .unwrap()
            .iter()
            .any(|i| i["act"] == "save"));
        assert_eq!(
            state.value(
                &f,
                Room {
                    width: 1196.,
                    height: 600.
                }
            ),
            v
        );
    }
    assert!(matches!(state.input(Input::Go(ids[6]), &f), Reply::Return));
    assert!(
        matches!(state.input(Input::Go(ids[0]),&f),Reply::Go(ChromeIntent::Restore(id)) if id==ids[0])
    );
}

#[test]
fn aggregate_memory_and_current_previous_marks_are_independent() {
    let (mut f, ids) = fixture(7, true, false);
    f.forest.show_current(ids[0]);
    let facts = Facts {
        forest: &f.forest,
        previous: Some(ids[6]),
        live: &f.live,
        playing: &f.playing,
        shown: &f.shown,
    };
    let folded = items::fold(&facts, &f.forest.tabs()[0], Level::Far);
    assert_eq!(folded[1].appearance.memory, items::Memory::Visited);
    assert!(folded[1].appearance.previous);
    assert!(folded[0].appearance.current);
    f.live.insert(ids[3]);
    let facts = Facts {
        forest: &f.forest,
        previous: Some(ids[6]),
        live: &f.live,
        playing: &f.playing,
        shown: &f.shown,
    };
    let folded = items::fold(&facts, &f.forest.tabs()[0], Level::Far);
    assert_eq!(folded[1].appearance.memory, items::Memory::Open);
    assert!(folded[1].appearance.previous);
}

#[test]
fn back_is_absent_in_a_tree_with_only_one_read_page() {
    let (f, _) = fixture(1, false, false);
    assert!(State::default().value(
        &f,
        Room {
            width: 1196.,
            height: 600.
        }
    )["back"]
        .is_null());
}

#[test]
fn islands_boxes_clear_at_minimum_and_rest_and_chains_stay_compact() {
    let chain = |n| (0..n).map(|i: usize| i.checked_sub(1)).collect::<Vec<_>>();
    for parents in [
        chain(90),
        chain(500),
        (0..500).map(|i| (i > 0).then_some(0)).collect(),
        (0..80)
            .map(|i| if i == 0 { None } else { Some((i - 1) / 3) })
            .collect(),
    ] {
        let (f, ids) = fixture_with_parents(&parents, false, false);
        let facts = Facts {
            forest: &f.forest,
            previous: Some(ids[0]),
            live: &f.live,
            playing: &f.playing,
            shown: &f.shown,
        };
        for level in Level::ALL {
            let mut folded = items::fold(&facts, &f.forest.tabs()[0], Level::Near);
            for item in &mut folded {
                item.named = true;
            }
            for width in [390., 728., 1920.] {
                let p = Layout::Islands.place(
                    level,
                    std::slice::from_ref(&folded),
                    Room {
                        width,
                        height: 800.,
                    },
                );
                for scale in [level.band().min / level.band().rest, 1.] {
                    for (i, a) in p.items[0].iter().enumerate() {
                        for (j, b) in p.items[0].iter().enumerate().skip(i + 1) {
                            assert!(
                                (a.x - b.x).abs() * scale
                                    >= (folded[i].width(level) + folded[j].width(level)) / 2.
                                    || a.y * scale + folded[i].extent(level).1
                                        <= b.y * scale - folded[j].extent(level).0
                                    || b.y * scale + folded[j].extent(level).1
                                        <= a.y * scale - folded[i].extent(level).0,
                                "Islands overlap {} {level:?} {i} {j}",
                                parents.len()
                            );
                        }
                    }
                }
                {
                    let root = p.items[0][0];
                    let max = p.items[0]
                        .iter()
                        .map(|v| (v.x - root.x).hypot(v.y - root.y))
                        .fold(0_f64, f64::max);
                    let unit = folded
                        .iter()
                        .map(|i| {
                            let (a, b) = i.extent(level);
                            i.width(level).hypot(2. * a.max(b)) + 20.
                        })
                        .fold(1_f64, f64::max)
                        * level.band().rest
                        / level.band().min;
                    assert!(
                        max <= 15. * unit,
                        "tree {} {level:?} needs more than fifteen rings, max {max} unit {unit}",
                        parents.len()
                    );
                    assert!(p.items[0].iter().any(|p| p.x < root.x));
                    assert!(p.items[0].iter().any(|p| p.x > root.x));
                }
            }
        }
    }
}

#[test]
fn islands_wraps_trees_at_every_resting_zoom() {
    let (f, _) = fixture(1, false, false);
    let facts = Facts {
        forest: &f.forest,
        previous: None,
        live: &f.live,
        playing: &f.playing,
        shown: &f.shown,
    };
    for level in Level::ALL {
        let trees = (0..20)
            .map(|_| items::fold(&facts, &f.forest.tabs()[0], level))
            .collect::<Vec<_>>();
        let p = Layout::Islands.place(
            level,
            &trees,
            Room {
                width: 1920.,
                height: 800.,
            },
        );
        assert!(p.trees[1].x > p.trees[0].x);
        assert!(p.trees.last().unwrap().y > p.trees[0].y);
    }
}

fn island_items(parents: &[Option<usize>]) -> Vec<items::Item> {
    let (mut f, ids) = fixture_with_parents(parents, false, false);
    f.forest.show_current(ids[0]);
    let facts = Facts {
        forest: &f.forest,
        previous: ids.last().copied(),
        live: &f.live,
        playing: &f.playing,
        shown: &f.shown,
    };
    items::fold(&facts, &f.forest.tabs()[0], Level::Near)
}
#[test]
fn islands_short_chain_is_one_outward_spoke() {
    let items = island_items(&[None, Some(0), Some(1)]);
    for level in Level::ALL {
        let (at, _) = place::island_points(&items, level);
        let a = at[1].y.atan2(at[1].x);
        let b = at[2].y.atan2(at[2].x);
        assert!((a - b).abs() < 0.01);
        assert!(at[1].x.hypot(at[1].y) < at[2].x.hypot(at[2].y));
    }
}
#[test]
fn islands_children_stay_in_their_parents_share_and_hubs_surround_the_root() {
    let parents: Vec<_> = (0..31)
        .map(|i| if i == 0 { None } else { Some((i - 1) / 5) })
        .collect();
    let items = island_items(&parents);
    for level in Level::ALL {
        let (at, shares) = place::island_points(&items, level);
        let mut angles = Vec::new();
        for (i, item) in items.iter().enumerate().skip(1) {
            let parent = item.parent.unwrap();
            assert!(shares[i].start >= shares[parent].start - 1e-9);
            assert!(shares[i].end <= shares[parent].end + 1e-9);
            let mut angle = at[i].y.atan2(at[i].x);
            while angle < shares[i].start {
                angle += std::f64::consts::TAU;
            }
            assert!(angle <= shares[i].end + 1e-9);
            if parent == 0 {
                angles.push(angle);
            }
        }
        angles.sort_by(f64::total_cmp);
        assert_eq!(angles.len(), 5);
        for i in 0..5 {
            let next = if i == 4 {
                angles[0] + std::f64::consts::TAU
            } else {
                angles[i + 1]
            };
            assert!(next - angles[i] < std::f64::consts::PI);
        }
    }
}
fn segment_hits_box(
    a: Point,
    b: Point,
    c: Point,
    item: &items::Item,
    level: Level,
    scale: f64,
) -> bool {
    let a = Point {
        x: a.x * scale,
        y: a.y * scale,
    };
    let b = Point {
        x: b.x * scale,
        y: b.y * scale,
    };
    let (above, below) = item.extent(level);
    let x = c.x * scale;
    let y = c.y * scale;
    let w = item.width(level) / 2.;
    let corners = [
        Point {
            x: x - w,
            y: y - above,
        },
        Point {
            x: x + w,
            y: y - above,
        },
        Point {
            x: x + w,
            y: y + below,
        },
        Point {
            x: x - w,
            y: y + below,
        },
    ];
    let inside = |p: Point| p.x >= x - w && p.x <= x + w && p.y >= y - above && p.y <= y + below;
    let cross =
        |p: Point, q: Point, r: Point| (q.x - p.x) * (r.y - p.y) - (q.y - p.y) * (r.x - p.x);
    inside(a)
        || inside(b)
        || (0..4).any(|i| {
            let c = corners[i];
            let d = corners[(i + 1) % 4];
            cross(a, b, c) * cross(a, b, d) <= 0.
                && cross(c, d, a) * cross(c, d, b) <= 0.
                && a.x.min(b.x) <= c.x.max(d.x)
                && c.x.min(d.x) <= a.x.max(b.x)
                && a.y.min(b.y) <= c.y.max(d.y)
                && c.y.min(d.y) <= a.y.max(b.y)
        })
}
#[test]
fn islands_edges_clear_unrelated_boxes_in_short_chains_hubs_and_branches() {
    let cases = [
        vec![None, Some(0), Some(1)],
        (0..12)
            .map(|i| match i {
                0 => None,
                1..=5 => Some(0),
                _ => Some(i - 1),
            })
            .collect(),
        (0..30)
            .map(|i| if i == 0 { None } else { Some((i - 1) / 3) })
            .collect(),
        (0..21)
            .map(|i| if i == 0 { None } else { Some(0) })
            .collect(),
    ];
    for parents in cases {
        let items = island_items(&parents);
        for level in Level::ALL {
            let (at, _) = place::island_points(&items, level);
            for scale in [1., level.band().min / level.band().rest] {
                for (i, item) in items.iter().enumerate() {
                    if let Some(parent) = item.parent {
                        for (j, other) in items.iter().enumerate() {
                            if j != i && j != parent {
                                assert!(
                                    !segment_hits_box(
                                        at[parent], at[i], at[j], other, level, scale
                                    ),
                                    "edge {parent}-{i} hits {j} in {} items at {level:?}",
                                    items.len()
                                );
                            }
                        }
                    }
                }
            }
        }
    }
}
#[test]
fn islands_thirty_item_tree_fits_the_middle_room() {
    let parents: Vec<_> = (0..30)
        .map(|i| if i == 0 { None } else { Some((i - 1) / 5) })
        .collect();
    let mut items = island_items(&parents);
    for item in &mut items {
        item.named = item.appearance.current || item.appearance.previous;
    }
    let room = Room {
        width: 1400.,
        height: 760.,
    };
    let p = Layout::Islands.place(Level::Middle, std::slice::from_ref(&items), room);
    let pan = place::camera(&p, Some((0, 0)), room, 1.);
    for (at, item) in p.items[0].iter().zip(&items) {
        assert!(at.x + pan[0] - item.width(Level::Middle) / 2. >= 0.);
        assert!(at.x + pan[0] + item.width(Level::Middle) / 2. <= room.width);
        assert!(at.y + pan[1] - item.extent(Level::Middle).0 >= 0.);
        assert!(
            at.y + pan[1] + item.extent(Level::Middle).1 <= room.height,
            "bottom {}",
            at.y + pan[1] + item.extent(Level::Middle).1
        );
    }
}

#[test]
fn islands_resting_spoke_spacing_uses_the_item_boxes() {
    let mut items = island_items(&[None, Some(0), Some(1)]);
    items[1].named = false;
    items[1].pages = vec![items[1].go; 8];
    for level in Level::ALL {
        let (at, _) = place::island_points(&items, level);
        let before = items
            .iter()
            .map(|i| {
                let (a, b) = i.extent(level);
                i.width(level).hypot(2. * a.max(b)) + 20.
            })
            .fold(1_f64, f64::max)
            * level.band().rest
            / level.band().min;
        let spacing = at[1].x.hypot(at[1].y);
        println!("spacing {level:?}: before {before:.3}, after {spacing:.3} CSS px at rest");
        let range = match level {
            Level::Far => 50. ..60.,
            Level::Middle => 110. ..150.,
            Level::Near => 230. ..260.,
        };
        assert!(range.contains(&spacing));
    }
}
#[test]
fn islands_coiled_chains_keep_their_edges_clear() {
    for n in [90, 500] {
        let items = island_items(&(0..n).map(|i: usize| i.checked_sub(1)).collect::<Vec<_>>());
        for level in Level::ALL {
            let (at, shares) = place::island_points(&items, level);
            for (i, item) in items.iter().enumerate().skip(1) {
                let parent = item.parent.unwrap();
                let mut angle = at[i].y.atan2(at[i].x);
                while angle < shares[parent].start {
                    angle += std::f64::consts::TAU;
                }
                assert!(angle <= shares[parent].end + 1e-9);
                for scale in [1., level.band().min / level.band().rest] {
                    for (j, other) in items.iter().enumerate() {
                        if j != i && j != parent {
                            assert!(
                                !segment_hits_box(at[parent], at[i], at[j], other, level, scale),
                                "chain {n} edge {parent}-{i} hits {j} {level:?}"
                            );
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn a_page_playing_sound_keeps_its_own_item_at_every_level() {
    let (mut f, ids) = fixture(7, false, false);
    f.playing = [ids[2]].into_iter().collect();
    let facts = Facts {
        forest: &f.forest,
        previous: None,
        live: &f.live,
        playing: &f.playing,
        shown: &f.shown,
    };
    let tree = &f.forest.tabs()[0];
    for level in Level::ALL {
        let items = items::fold(&facts, tree, level);
        let playing: Vec<_> = items.iter().filter(|i| i.appearance.playing).collect();
        assert_eq!(playing.len(), 1, "{level:?}");
        assert_eq!(playing[0].pages, [ids[2]], "{level:?}");
        let value = playing[0].paint(&facts, level);
        assert_eq!(value["playing"], true);
        assert!(value["tip"].as_str().unwrap().contains("playing sound"));
        assert!(items
            .iter()
            .filter(|i| !i.appearance.playing)
            .all(|i| !i.pages.contains(&ids[2])));
    }
}
