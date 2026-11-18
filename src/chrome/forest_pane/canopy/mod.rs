mod items;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Level {
    Far,
    Middle,
    Near,
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        browser::{Browser, VisitId},
        chrome::ChromeFrame,
        forest::ForestProjection,
    };
    use items::Facts;
    fn fixture(n: usize, two_sites: bool, waiting: bool) -> (ChromeFrame, Vec<VisitId>) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let mut b = Browser::open(&path).unwrap();
        let root = b
            .add_roots(&[url::Url::parse("https://hub.test/").unwrap()])
            .unwrap()[0];
        let db = rusqlite::Connection::open(&path).unwrap();
        let mut ids = vec![root];
        for i in 1..n {
            let id: VisitId = ulid::Ulid::new().to_string().parse().unwrap();
            db.execute(
            "INSERT INTO visits(id,parent_id,url,title,created_at,opened_at) VALUES(?,?,?,?,?,?)",
            rusqlite::params![
                id.to_string(),
                ids[i - 1].to_string(),
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
        for _ in 0..5 {
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
            ForestProjection::from_snapshot(b.snapshot().unwrap(), b.tree_states().unwrap())
                .unwrap();
        let mut frame = super::super::tests::fixture(1, 1);
        frame.forest = forest;
        frame.shown = b.showings().unwrap();
        (frame, ids)
    }
    #[test]
    fn fold_at_three_levels_keeps_hub_and_chain_targets() {
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
        let mid = items::fold(&facts, tree, Level::Middle);
        assert_eq!(
            mid.iter().map(|i| i.pages.len()).collect::<Vec<_>>(),
            [1, 5, 1]
        );
        assert!(mid[0].appearance.previous && mid[2].appearance.current);
        assert_eq!(items::fold(&facts, tree, Level::Near).len(), 7);
        assert_eq!(items::size(1, 20.), 20.);
        assert_eq!(items::size(u32::MAX, 20.), 42.);
    }
}
