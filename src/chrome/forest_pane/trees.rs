use super::super::view::plural;
use super::*;
use crate::browser::{Move, Place, Slot};
use crate::forest::ForestProjection;

#[derive(Default)]
pub(super) struct State {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::chrome) enum Lane {
    Tabs,
    Shelf,
    Settled,
}
impl Lane {
    const ALL: [Self; 3] = [Self::Tabs, Self::Shelf, Self::Settled];
    fn name(self) -> &'static str {
        match self {
            Self::Tabs => "tabs",
            Self::Shelf => "shelf",
            Self::Settled => "settled",
        }
    }
    fn of(place: Place) -> Self {
        match place {
            Place::Tab { .. } => Self::Tabs,
            Place::Shelf { .. } => Self::Shelf,
            Place::History { .. } => Self::Settled,
        }
    }
}
#[derive(Debug)]
pub(in crate::chrome) enum Input {
    Drop {
        tree: TreeId,
        lane: Lane,
        before: Option<TreeId>,
    },
}
impl Input {
    pub fn parse(m: &Value) -> Result<Option<Self>> {
        Ok(match m["do"].as_str() {
            Some("drop") => Some(Self::Drop {
                tree: TreeId(
                    m["tree"]
                        .as_str()
                        .context("Missing dragged tree")?
                        .parse()?,
                ),
                lane: Lane::ALL
                    .into_iter()
                    .find(|l| Some(l.name()) == m["lane"].as_str())
                    .context("Unknown lane")?,
                before: if m["before"].is_null() {
                    None
                } else {
                    Some(TreeId(
                        m["before"]
                            .as_str()
                            .context("Invalid drop position")?
                            .parse()?,
                    ))
                },
            }),
            _ => None,
        })
    }
}
fn drop_move(from: Place, lane: Lane, before: Option<TreeId>) -> Option<Move> {
    let at = before.map_or(Slot::End, Slot::Before);
    match (from, lane) {
        (Place::History { .. }, Lane::Settled) => None,
        (_, Lane::Settled) => Some(Move::History),
        (_, Lane::Tabs) => Some(Move::Tabs(at)),
        (_, Lane::Shelf) => Some(Move::Shelf(at)),
    }
}
fn accepts(target: Lane) -> Vec<&'static str> {
    [
        Place::Tab { pinned: false },
        Place::Shelf { saved_at: 0 },
        Place::History { settled_at: 0 },
    ]
    .into_iter()
    .filter(|from| drop_move(*from, target, None).is_some())
    .map(|from| Lane::of(from).name())
    .collect()
}
impl State {
    pub fn query(&self) -> &str {
        ""
    }
    pub fn escape(&mut self) -> bool {
        false
    }
    pub fn input(
        &mut self,
        input: Input,
        forest: &ForestProjection,
        _calendar: &dyn Calendar,
    ) -> Reply {
        let Input::Drop { tree, lane, before } = input;
        let Some(source) = forest.tree(tree) else {
            return Reply::Unchanged;
        };
        if before.is_some_and(|id| {
            id == tree || forest.tree(id).is_none_or(|t| Lane::of(t.place) != lane)
        }) {
            return Reply::Unchanged;
        }
        drop_move(source.place, lane, before).map_or(Reply::Unchanged, |to| {
            Reply::Act(super::super::ChromeIntent::PlaceTree { tree, to })
        })
    }
    pub fn value(
        &self,
        forest: &ForestProjection,
        _index: &Index,
        calendar: &dyn Calendar,
    ) -> Value {
        let current = forest.current().and_then(|id| forest.tree_of(id));
        let today = calendar.now();
        let tabs = forest
            .tabs()
            .iter()
            .map(|t| tree_card::tab_card(forest, t, current == Some(t.id)))
            .collect::<Vec<_>>();
        let shelf = forest
            .shelf()
            .iter()
            .map(|t| {
                let mut c = tree_card::shelf_card(forest, t);
                c["kind"] = json!("card");
                c["waiting"] = json!(0);
                c["buttons"] = tree_card::buttons(t, &[Act::Open, Act::Settle]);
                if let Place::Shelf { saved_at } = t.place {
                    c["meta"] = json!(format!(
                        "{} · saved {}",
                        plural(t.page_count, "page"),
                        calendar.civil(saved_at).when(&today)
                    ));
                }
                c
            })
            .collect::<Vec<_>>();
        let settled = forest
            .history()
            .iter()
            .filter_map(|t| {
                let Place::History { settled_at } = t.place else {
                    return None;
                };
                let day = calendar.civil(settled_at);
                (day.date >= today.date).then(|| tree_card::settled_line(forest, t, &day.clock))
            })
            .collect::<Vec<_>>();
        let lanes = [
            (Lane::Tabs, "Tabs", tabs, "No open tab."),
            (Lane::Shelf, "Saved for later", shelf, "Drop a tab here to put it aside."),
            (Lane::Settled, "Settled today", settled, "Nothing settled today."),
        ].into_iter().map(|(lane,title,mut cards,empty)| {
            for c in &mut cards { c["key"] = json!(format!("tree-{}", c["id"].as_str().unwrap())); }
            json!({"key":if lane == Lane::Shelf { "saved-lane".to_owned() } else { format!("lane-{}",lane.name()) },"lane":lane.name(),"title":title,"count":plural(cards.len(),if lane == Lane::Tabs {"tab"}else{"tree"}),"accepts":accepts(lane),"ordered":lane != Lane::Settled,"cards":cards,"empty":empty,"foot":(lane==Lane::Settled).then(||json!({"label":"All of History","view":"history"}))})
        }).collect::<Vec<_>>();
        json!({"lanes":lanes})
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{change, fixture};
    use super::*;

    fn id(n: u128) -> TreeId {
        TreeId(ulid::Ulid::from(n).to_string().parse().unwrap())
    }
    #[test]
    fn whole_drop_table_and_accepts() {
        for from in [
            Place::Tab { pinned: false },
            Place::Tab { pinned: true },
            Place::Shelf { saved_at: 1 },
            Place::History { settled_at: 1 },
        ] {
            for lane in Lane::ALL {
                for before in [None, Some(id(10))] {
                    let slot = before.map_or(Slot::End, Slot::Before);
                    let expected = match lane {
                        Lane::Tabs => Some(Move::Tabs(slot)),
                        Lane::Shelf => Some(Move::Shelf(slot)),
                        Lane::Settled if matches!(from, Place::History { .. }) => None,
                        Lane::Settled => Some(Move::History),
                    };
                    assert_eq!(drop_move(from, lane, before), expected);
                    assert_eq!(
                        accepts(lane).contains(&Lane::of(from).name()),
                        expected.is_some()
                    );
                }
            }
        }
    }
    #[test]
    fn drops_reject_stale_wrong_lane_and_self_anchors() {
        let f = fixture(3, 2);
        let tree = f.forest.tabs()[0].id;
        let history = f.forest.history()[0].id;
        let mut state = State::default();
        for before in [id(999), history, tree] {
            assert!(matches!(
                state.input(
                    Input::Drop {
                        tree,
                        lane: Lane::Tabs,
                        before: Some(before)
                    },
                    &f.forest,
                    &LocalCalendar
                ),
                Reply::Unchanged
            ));
        }
        assert!(matches!(
            state.input(
                Input::Drop {
                    tree: id(999),
                    lane: Lane::Shelf,
                    before: None
                },
                &f.forest,
                &LocalCalendar
            ),
            Reply::Unchanged
        ));
        assert!(matches!(
            state.input(
                Input::Drop {
                    tree,
                    lane: Lane::Shelf,
                    before: None
                },
                &f.forest,
                &LocalCalendar
            ),
            Reply::Act(super::super::super::ChromeIntent::PlaceTree {
                to: Move::Shelf(Slot::End),
                ..
            })
        ));
    }
    #[test]
    fn drop_input_is_typed_and_trees_has_no_find() {
        for m in [
            json!({"do":"drop","tree":"bad","lane":"tabs"}),
            json!({"do":"drop","tree":id(1).root().to_string(),"lane":"unknown"}),
            json!({"do":"drop","tree":id(1).root().to_string(),"lane":"tabs","before":17}),
        ] {
            assert!(Input::parse(&m).is_err());
        }
        assert!(Input::parse(&json!({"do":"trees-find","query":"x"}))
            .unwrap()
            .is_none());
        assert_eq!(State::default().query(), "");
        assert!(!State::default().escape());
    }
    #[test]
    fn lane_values_show_tabs_shelf_and_all_of_todays_history() {
        let mut f = fixture(14, 3);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        change(&mut f, |states| {
            states.sort_by_key(|s| s.root.root().to_string());
            for (i, s) in states.iter_mut().enumerate() {
                s.spot = match i {
                    0..=1 => crate::browser::Spot::Tab {
                        pinned: i == 0,
                        position: i as u32,
                    },
                    2 => crate::browser::Spot::Shelf {
                        saved_at: now,
                        position: 0,
                    },
                    _ => crate::browser::Spot::History {
                        settled_at: now - i as i64 * 1000,
                    },
                };
            }
        });
        let index = Index::build(&f.forest, &LocalCalendar);
        let v = State::default().value(&f.forest, &index, &LocalCalendar);
        let lanes = v["lanes"].as_array().unwrap();
        assert_eq!(lanes.len(), 3);
        assert_eq!(lanes[0]["title"], "Tabs");
        assert_eq!(lanes[0]["count"], "2 tabs");
        assert_eq!(lanes[0]["cards"][0]["kind"], "line");
        assert_eq!(lanes[0]["cards"][1]["kind"], "card");
        for c in lanes[0]["cards"].as_array().unwrap() {
            assert_eq!(c["buttons"].as_array().unwrap().len(), 3);
            assert!(c["mini"]["nodes"].is_array());
            assert!(c["page"].is_string());
            assert_eq!(c["meta"], "3 pages");
        }
        assert_eq!(lanes[0]["cards"][0]["current"], true);
        assert_eq!(lanes[1]["title"], "Saved for later");
        assert!(lanes[1]["cards"][0]["meta"]
            .as_str()
            .unwrap()
            .contains("3 pages · saved today at"));
        assert_eq!(lanes[1]["cards"][0]["buttons"][0]["label"], "Open");
        assert_eq!(lanes[1]["cards"][0]["buttons"][1]["act"], "settle");
        assert_eq!(lanes[2]["cards"].as_array().unwrap().len(), 11);
        assert_eq!(lanes[2]["count"], "11 trees");
        assert_eq!(
            lanes[2]["cards"][0]["id"],
            f.forest.history()[0].id.root().to_string()
        );
        assert_eq!(lanes[2]["cards"][0]["buttons"][0]["label"], "Reopen");
        assert_eq!(lanes[2]["cards"][0]["kind"], "line");
        assert_eq!(lanes[2]["foot"]["view"], "history");
        assert!(v["query"].is_null());
        let f = fixture(1, 1);
        let v = State::default().value(
            &f.forest,
            &Index::build(&f.forest, &LocalCalendar),
            &LocalCalendar,
        );
        assert!(v["lanes"][2]["cards"].as_array().unwrap().is_empty());
        assert_eq!(v["lanes"][2]["empty"], "Nothing settled today.");
    }
    #[test]
    fn keyboard_step_never_crosses_the_pin_border_or_reorders_history() {
        let dir = tempfile::tempdir().unwrap();
        let mut b = crate::browser::Browser::open(&dir.path().join("db")).unwrap();
        let roots = b
            .add_roots(
                &(0..6)
                    .map(|n| url::Url::parse(&format!("https://example.org/{n}")).unwrap())
                    .collect::<Vec<_>>(),
            )
            .unwrap();
        let trees = roots.into_iter().map(TreeId).collect::<Vec<_>>();
        b.place_tree(trees[0], Move::Pin(true)).unwrap();
        b.place_tree(trees[1], Move::Pin(true)).unwrap();
        b.place_tree(trees[4], Move::Shelf(Slot::End)).unwrap();
        b.place_tree(trees[5], Move::History).unwrap();
        assert_eq!(b.tree_step(trees[1], true).unwrap(), None);
        assert_eq!(b.tree_step(trees[2], false).unwrap(), None);
        assert_eq!(b.tree_step(trees[4], true).unwrap(), None);
        assert_eq!(b.tree_step(trees[5], false).unwrap(), None);
        let to = b.tree_step(trees[0], true).unwrap().unwrap();
        b.place_tree(trees[0], to).unwrap();
        let states = b.tree_states().unwrap();
        assert_eq!(states[0].root, trees[1]);
        assert_eq!(states[1].root, trees[0]);
        assert!(states[1].place().pinned());
    }
}
