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
            shown: &f.shown,
        };
        for level in Level::ALL {
            // Different sites prevent the low zooms from hiding geometry behind folding.
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
                if parents[2] == Some(1) {
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
                    assert!(max <= 15. * unit, "chain became a spoke");
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
