use super::{now_ms, Browser, Landing, TreeId, VisitId};
use anyhow::{bail, Context, Result};
use rusqlite::{params, OptionalExtension, Transaction, TransactionBehavior};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Place {
    Tab { pinned: bool },
    Shelf { saved_at: i64 },
    History { settled_at: i64 },
}
impl Place {
    pub fn is_tab(self) -> bool {
        matches!(self, Self::Tab { .. })
    }
    pub fn pinned(self) -> bool {
        matches!(self, Self::Tab { pinned: true })
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Spot {
    Tab { pinned: bool, position: u32 },
    Shelf { position: u32, saved_at: i64 },
    History { settled_at: i64 },
}
impl Spot {
    pub fn place(self) -> Place {
        match self {
            Self::Tab { pinned, .. } => Place::Tab { pinned },
            Self::Shelf { saved_at, .. } => Place::Shelf { saved_at },
            Self::History { settled_at } => Place::History { settled_at },
        }
    }
    pub(super) fn from_row(
        place: &str,
        position: Option<i64>,
        pinned: bool,
        at: i64,
    ) -> Result<Self> {
        Ok(match (place, position, pinned) {
            ("tab", Some(p), pinned) => Self::Tab {
                pinned,
                position: p.try_into()?,
            },
            ("shelf", Some(p), false) => Self::Shelf {
                position: p.try_into()?,
                saved_at: at,
            },
            ("history", None, false) => Self::History { settled_at: at },
            _ => bail!("Invalid tree place."),
        })
    }
    pub(crate) fn order(self) -> (u8, bool, u32, std::cmp::Reverse<i64>) {
        match self {
            Self::Tab { pinned, position } => (0, !pinned, position, std::cmp::Reverse(0)),
            Self::Shelf { position, .. } => (1, false, position, std::cmp::Reverse(0)),
            Self::History { settled_at } => (2, false, 0, std::cmp::Reverse(settled_at)),
        }
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Slot {
    First,
    Before(TreeId),
    End,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Move {
    /// Opening an existing tab preserves its position. Reopening appends an unpinned tab.
    Open,
    Tabs(Slot),
    Pin(bool),
    Shelf(Slot),
    History,
}
pub(crate) fn pin_after_drop(left: Option<bool>, right: Option<bool>, was: bool) -> bool {
    match (left, right) {
        (_, Some(true)) => true,
        (Some(false), _) => false,
        _ => was,
    }
}
pub(super) const ORDER: &str = "CASE place WHEN 'tab' THEN 0 WHEN 'shelf' THEN 1 ELSE 2 END, pinned DESC, position, placed_at DESC, root_id";
impl Browser {
    pub fn place_tree(&mut self, tree: TreeId, to: Move) -> Result<Landing> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        place_in(&tx, tree, to, now_ms()?)?;
        let landing = match to {
            Move::Shelf(_) | Move::History => land(&tx)?,
            _ => Landing::Unchanged,
        };
        tx.commit()?;
        Ok(landing)
    }
    pub fn resume_landing(&mut self) -> Result<Landing> {
        let tx = self.connection.transaction()?;
        let landing = land(&tx)?;
        tx.commit()?;
        Ok(landing)
    }
}
pub(super) fn append_tab(tx: &Transaction, root: VisitId, last: VisitId, at: i64) -> Result<()> {
    tx.execute("INSERT OR IGNORE INTO tree_state(root_id,last_visit_id,place,position,pinned,placed_at) VALUES(?1,?2,'tab',(SELECT COALESCE(MAX(position),-1)+1 FROM tree_state WHERE place='tab'),0,?3)", params![root.to_string(),last.to_string(),at])?;
    Ok(())
}
pub(super) fn place_in(tx: &Transaction, tree: TreeId, to: Move, now: i64) -> Result<()> {
    let (from, pin, at): (String, bool, i64) = tx
        .query_row(
            "SELECT place,pinned,placed_at FROM tree_state WHERE root_id=?1",
            [tree.root().to_string()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?
        .context("This tree no longer exists.")?;
    let target = match to {
        Move::Open if from == "tab" => return Ok(()),
        Move::Pin(p) if from != "tab" || pin == p => return Ok(()),
        Move::History if from == "history" => return Ok(()),
        Move::History => {
            tx.execute("UPDATE tree_state SET place='history',position=NULL,pinned=0,placed_at=?2 WHERE root_id=?1",params![tree.root().to_string(),now])?;
            return Ok(());
        }
        Move::Shelf(_) => "shelf",
        _ => "tab",
    };
    let mut stmt = tx.prepare(&format!(
        "SELECT root_id,pinned FROM tree_state WHERE place=?1 AND root_id<>?2 ORDER BY {ORDER}"
    ))?;
    let mut lane = stmt
        .query_map(params![target, tree.root().to_string()], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, bool>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let slot = match to {
        Move::Tabs(s) | Move::Shelf(s) => s,
        _ => Slot::End,
    };
    let i = match to {
        Move::Pin(_) => lane.partition_point(|(_, p)| *p),
        _ => match slot {
            Slot::First => 0,
            Slot::End => lane.len(),
            Slot::Before(t) => lane
                .iter()
                .position(|(id, _)| *id == t.root().to_string())
                .unwrap_or(lane.len()),
        },
    };
    let pinned = match to {
        Move::Pin(p) => p,
        Move::Tabs(_) => pin_after_drop(
            i.checked_sub(1).map(|j| lane[j].1),
            lane.get(i).map(|x| x.1),
            pin,
        ),
        _ => false,
    };
    lane.insert(i, (tree.root().to_string(), pinned));
    tx.execute(
        "UPDATE tree_state SET place=?2,position=0,pinned=?3,placed_at=?4 WHERE root_id=?1",
        params![
            tree.root().to_string(),
            target,
            pinned,
            if from == target { at } else { now }
        ],
    )?;
    for (i, (id, pin)) in lane.iter().enumerate() {
        tx.execute(
            "UPDATE tree_state SET position=?2,pinned=?3 WHERE root_id=?1",
            params![id, i as i64, pin],
        )?;
    }
    Ok(())
}

fn land(tx: &Transaction) -> Result<Landing> {
    // A window already resting in a settled tree, after a landing that failed or has not
    // committed, gets a landing again whichever tree is settled now.
    let stranded: Option<bool> = tx.query_row(
            "WITH RECURSIVE ancestry(id, parent_id) AS (
                SELECT id, parent_id FROM visits WHERE id=(SELECT current_id FROM session WHERE singleton=1)
                UNION
                SELECT v.id, v.parent_id FROM visits v JOIN ancestry a ON v.id=a.parent_id
             ) SELECT t.place <> 'tab' FROM ancestry a JOIN tree_state t ON t.root_id=a.id
               WHERE a.parent_id IS NULL",
            [], |row| row.get(0),
        ).optional()?;
    if stranded != Some(true) {
        return Ok(Landing::Unchanged);
    }
    let next: Option<String> = tx.query_row(
            "SELECT t.last_visit_id FROM tree_state t JOIN visits r ON r.id=t.root_id
             WHERE t.place = 'tab'
             ORDER BY (SELECT MAX(e.id) FROM visit_events e WHERE e.visit_id=t.last_visit_id) IS NULL,
                      (SELECT MAX(e.id) FROM visit_events e WHERE e.visit_id=t.last_visit_id) DESC,
                      r.created_at DESC, r.rowid DESC LIMIT 1",
            [], |row| row.get(0),
        ).optional()?;
    if let Some(next) = next {
        let visit = next.parse()?;
        Ok(Landing::Restore(visit))
    } else {
        tx.execute("UPDATE session SET current_id=NULL WHERE singleton=1", [])?;
        Ok(Landing::Start)
    }
}

pub(super) fn create_tree_state_triggers(transaction: &Transaction) -> Result<()> {
    for action in ["INSERT", "UPDATE"] {
        transaction.execute_batch(&format!(
                    "CREATE TRIGGER tree_state_validate_{action} BEFORE {action} ON tree_state
                     BEGIN
                        SELECT RAISE(ABORT, 'Invalid tree target') WHERE NOT EXISTS (
                            WITH RECURSIVE ancestry(id, parent_id, opened_at) AS (
                                SELECT id, parent_id, opened_at FROM visits WHERE id=NEW.last_visit_id
                                UNION
                                SELECT v.id, v.parent_id, v.opened_at FROM visits v
                                    JOIN ancestry a ON v.id=a.parent_id
                            )
                            SELECT 1 FROM ancestry WHERE id=NEW.root_id AND parent_id IS NULL
                                AND (SELECT opened_at FROM visits WHERE id=NEW.last_visit_id) IS NOT NULL
                        );
                     END;"
                ))?;
    }

    Ok(())
}
pub(super) const MIGRATE_15: &str = "
        CREATE TABLE tree_state_next (
            root_id TEXT PRIMARY KEY NOT NULL REFERENCES visits(id),
            last_visit_id TEXT NOT NULL REFERENCES visits(id),
            name TEXT,
            place TEXT NOT NULL CHECK(place IN ('tab', 'shelf', 'history')),
            position INTEGER CHECK(position >= 0),
            pinned INTEGER NOT NULL DEFAULT 0 CHECK(pinned IN (0, 1)),
            placed_at INTEGER NOT NULL,
            CHECK((place = 'history') = (position IS NULL)),
            CHECK(pinned = 0 OR place = 'tab')
        );
        WITH old AS (
            SELECT t.root_id, t.last_visit_id, t.name, t.settled_at, t.pin_order,
                   r.created_at, r.rowid AS root_rowid,
                   CASE WHEN t.settled_at IS NULL THEN 'tab'
                        WHEN t.pin_order IS NOT NULL THEN 'shelf'
                        ELSE 'history' END AS place
            FROM tree_state t JOIN visits r ON r.id = t.root_id
        )
        INSERT INTO tree_state_next(root_id, last_visit_id, name, place, position, pinned, placed_at)
        SELECT root_id, last_visit_id, name, place,
               CASE place
                   WHEN 'tab' THEN row_number() OVER (PARTITION BY place
                       ORDER BY pin_order IS NULL, created_at, root_rowid) - 1
                   WHEN 'shelf' THEN row_number() OVER (PARTITION BY place ORDER BY pin_order DESC) - 1
               END,
               place = 'tab' AND pin_order IS NOT NULL,
               COALESCE(settled_at, created_at)
        FROM old;
        DROP TABLE tree_state;
        ALTER TABLE tree_state_next RENAME TO tree_state;
    ";

pub(super) fn migrate_15(tx: &Transaction) -> Result<()> {
    tx.execute_batch(MIGRATE_15)?;
    create_tree_state_triggers(tx)?;
    anyhow::ensure!(
        !tx.prepare("PRAGMA foreign_key_check")?.exists([])?,
        "Foreign key violation during forest upgrade."
    );
    tx.execute_batch("PRAGMA user_version = 15;")?;
    Ok(())
}

#[cfg(test)]
pub(super) fn legacy_14(c: &rusqlite::Connection) {
    c.execute_batch("DROP TRIGGER tree_state_validate_INSERT; DROP TRIGGER tree_state_validate_UPDATE;
        ALTER TABLE tree_state RENAME TO places_15;
        CREATE TABLE tree_state(root_id TEXT PRIMARY KEY NOT NULL REFERENCES visits(id),last_visit_id TEXT NOT NULL REFERENCES visits(id),name TEXT,settled_at INTEGER CHECK(settled_at>0),pin_order INTEGER CHECK(pin_order>0));
        INSERT INTO tree_state SELECT root_id,last_visit_id,name,CASE WHEN place='tab' THEN NULL ELSE placed_at END,CASE WHEN pinned=1 OR place='shelf' THEN position+1 END FROM places_15;
        DROP TABLE places_15;") .unwrap();
    let tx = c.unchecked_transaction().unwrap();
    create_tree_state_triggers(&tx).unwrap();
    tx.commit().unwrap();
}
#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    fn browser() -> Browser {
        Browser::with_connection(Connection::open_in_memory().unwrap()).unwrap()
    }
    fn roots(b: &mut Browser, n: usize) -> Vec<TreeId> {
        b.add_roots(
            &(0..n)
                .map(|i| url::Url::parse(&format!("https://example.org/{i}")).unwrap())
                .collect::<Vec<_>>(),
        )
        .unwrap()
        .into_iter()
        .map(TreeId)
        .collect()
    }
    #[test]
    fn migration_14_maps_each_combination_once() {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(include_str!("../../tests/fixtures/schema-v14.sql"))
            .unwrap();
        let ids = (1..=5)
            .map(|i| ulid::Ulid::from(i as u128).to_string())
            .collect::<Vec<_>>();
        for (i, id) in ids.iter().enumerate() {
            c.execute("INSERT INTO visits(id,url,created_at,opened_at) VALUES(?1,'https://example.org',1,1)",[id]).unwrap();
            c.execute("INSERT INTO tree_state(root_id,last_visit_id,settled_at,pin_order) VALUES(?1,?1,?2,?3)",params![id,if i>=2 {Some(2)}else{None},match i {1=>Some(1),3=>Some(2),4=>Some(3),_=>None}]).unwrap();
        }
        let b = Browser::with_connection(c).unwrap();
        let states = b.tree_states().unwrap();
        assert_eq!(
            states
                .iter()
                .map(|s| s.root.root().to_string())
                .collect::<Vec<_>>(),
            [1, 0, 4, 3, 2].map(|i| ids[i].clone())
        );
        assert_eq!(
            states.iter().map(|s| s.place()).collect::<Vec<_>>(),
            vec![
                Place::Tab { pinned: true },
                Place::Tab { pinned: false },
                Place::Shelf { saved_at: 2 },
                Place::Shelf { saved_at: 2 },
                Place::History { settled_at: 2 }
            ]
        );
        let b = Browser::with_connection(b.connection).unwrap();
        assert_eq!(states, b.tree_states().unwrap());
        assert_eq!(
            b.connection
                .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            15
        );
        assert_eq!(
            b.connection
                .query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))
                .unwrap(),
            "ok"
        );
        assert!(!b
            .connection
            .prepare("PRAGMA foreign_key_check")
            .unwrap()
            .exists([])
            .unwrap());
    }
    #[test]
    fn places_verbs_converge_and_reopening_appends_unpinned() {
        let mut b = browser();
        let t = roots(&mut b, 4);
        for to in [
            Move::Pin(true),
            Move::Pin(false),
            Move::Tabs(Slot::Before(t[0])),
            Move::Shelf(Slot::First),
            Move::Shelf(Slot::End),
            Move::History,
            Move::Open,
        ] {
            b.place_tree(t[2], to).unwrap();
            let once = b.tree_states().unwrap();
            b.place_tree(t[2], to).unwrap();
            assert_eq!(once, b.tree_states().unwrap());
        }
        assert_eq!(b.tree_states().unwrap().last().unwrap().root, t[2]);
        b.place_tree(t[0], Move::Pin(true)).unwrap();
        b.place_tree(t[0], Move::Shelf(Slot::First)).unwrap();
        assert!(matches!(
            b.tree_states().unwrap().last().unwrap().place(),
            Place::Shelf { .. }
        ));
        b.place_tree(t[0], Move::Open).unwrap();
        assert_eq!(
            b.tree_states().unwrap().last().unwrap().place(),
            Place::Tab { pinned: false }
        );
        for sql in [
            "UPDATE tree_state SET place='shelf',pinned=1",
            "UPDATE tree_state SET place='history',position=1",
        ] {
            assert!(b.connection.execute(sql, []).is_err());
        }
    }
    #[test]
    fn pin_after_drop_table() {
        for was in [false, true] {
            for left in [None, Some(false), Some(true)] {
                for right in [None, Some(false), Some(true)] {
                    assert_eq!(
                        pin_after_drop(left, right, was),
                        if right == Some(true) {
                            true
                        } else if left == Some(false) {
                            false
                        } else {
                            was
                        }
                    );
                }
            }
        }
    }
}
