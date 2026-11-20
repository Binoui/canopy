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
    #[cfg(test)]
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
    pub fn tree_step(&self, tree: TreeId, next: bool) -> Result<Option<Move>> {
        let states = self.tree_states()?;
        let Some(state) = states.iter().find(|s| s.root == tree) else {
            return Ok(None);
        };
        let lane = states
            .iter()
            .filter(|s| match (state.place(), s.place()) {
                (Place::Tab { pinned: a }, Place::Tab { pinned: b }) => a == b,
                (Place::Shelf { .. }, Place::Shelf { .. }) => true,
                _ => false,
            })
            .map(|s| s.root)
            .collect::<Vec<_>>();
        let Some(index) = lane.iter().position(|id| *id == tree) else {
            return Ok(None);
        };
        let slot = if next {
            if index + 1 == lane.len() {
                return Ok(None);
            }
            let after = states
                .iter()
                .position(|s| s.root == lane[index + 1])
                .unwrap()
                + 1;
            states
                .get(after)
                .filter(|s| {
                    matches!(
                        (state.place(), s.place()),
                        (Place::Tab { .. }, Place::Tab { .. })
                            | (Place::Shelf { .. }, Place::Shelf { .. })
                    )
                })
                .map_or(Slot::End, |s| Slot::Before(s.root))
        } else {
            if index == 0 {
                return Ok(None);
            }
            Slot::Before(lane[index - 1])
        };
        Ok(Some(if state.place().is_tab() {
            Move::Tabs(slot)
        } else {
            Move::Shelf(slot)
        }))
    }

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
pub(super) fn compact_tabs(tx: &Transaction) -> Result<()> {
    tx.execute(
        &format!(
            "WITH ordered AS MATERIALIZED (
                 SELECT root_id, ROW_NUMBER() OVER (ORDER BY {ORDER}) - 1 AS position
                 FROM tree_state WHERE place='tab'
             )
             UPDATE tree_state SET position=ordered.position FROM ordered
             WHERE tree_state.root_id=ordered.root_id AND tree_state.position<>ordered.position"
        ),
        [],
    )?;
    Ok(())
}

pub(super) fn place_in(tx: &Transaction, tree: TreeId, to: Move, now: i64) -> Result<()> {
    if matches!(to, Move::Tabs(Slot::Before(t)) | Move::Shelf(Slot::Before(t)) if t == tree) {
        return Ok(());
    }
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
pub(super) const MIGRATE_16: &str = "
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
            SELECT t.root_id, t.last_visit_id, t.name, t.settled_at, t.pin_order, t.tab_order,
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
                       ORDER BY pin_order IS NULL, tab_order) - 1
                   WHEN 'shelf' THEN row_number() OVER (PARTITION BY place ORDER BY pin_order DESC) - 1
               END,
               place = 'tab' AND pin_order IS NOT NULL,
               COALESCE(settled_at, created_at)
        FROM old;
        DROP TABLE tree_state;
        ALTER TABLE tree_state_next RENAME TO tree_state;
    ";

pub(super) fn migrate_16(tx: &Transaction) -> Result<()> {
    tx.execute_batch(MIGRATE_16)?;
    create_tree_state_triggers(tx)?;
    anyhow::ensure!(
        !tx.prepare("PRAGMA foreign_key_check")?.exists([])?,
        "Foreign key violation during forest upgrade."
    );
    tx.execute_batch("PRAGMA user_version = 16;")?;
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DeleteId(ulid::Ulid);
impl std::fmt::Display for DeleteId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}
impl std::str::FromStr for DeleteId {
    type Err = ulid::DecodeError;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        s.parse().map(Self)
    }
}

pub struct Deleted {
    id: DeleteId,
    /// Trees deleted whole.
    trees: Vec<TreeId>,
    /// Each deleted page with no deleted parent, and the parent it hung from.
    heads: Vec<(VisitId, Option<VisitId>)>,
    visits: std::collections::HashSet<VisitId>,
    tables: Vec<Rows>,
    nulled: Vec<(i64, String)>,
    declined: Vec<String>,
    /// Trees whose last page was deleted: (that page, the page they resume at now).
    retargeted: Vec<(VisitId, VisitId)>,
}
impl Deleted {
    pub fn id(&self) -> DeleteId {
        self.id
    }
    pub fn trees(&self) -> &[TreeId] {
        &self.trees
    }
    pub fn pages(&self) -> usize {
        self.visits.len()
    }
    pub fn holds(&self, id: VisitId) -> bool {
        self.visits.contains(&id)
    }
}
struct Rows {
    table: String,
    columns: Vec<String>,
    values: Vec<Vec<rusqlite::types::Value>>,
}
impl Rows {
    fn read(tx: &Transaction, table: &str, predicate: &str) -> Result<Self> {
        let mut stmt = tx.prepare(&format!(
            "SELECT rowid,* FROM {table} WHERE {predicate} ORDER BY rowid"
        ))?;
        let columns = stmt
            .column_names()
            .iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>();
        let values = stmt
            .query_map([], |r| (0..columns.len()).map(|i| r.get(i)).collect())?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(Self {
            table: table.to_owned(),
            columns,
            values,
        })
    }
    fn restore(&self, tx: &Transaction) -> Result<()> {
        use rusqlite::types::Value;
        // No foreign key refers to these integer keys. The schema coverage test checks that.
        let reuse = self
            .values
            .iter()
            .try_fold(false, |taken, row| -> Result<bool> {
                Ok(taken
                    || tx.query_row(
                        &format!("SELECT EXISTS(SELECT 1 FROM {} WHERE rowid=?1)", self.table),
                        [&row[0]],
                        |r| r.get::<_, bool>(0),
                    )?)
            })?;
        let integer_id = matches!(
            self.table.as_str(),
            "captures" | "downloads" | "visit_events"
        );
        let columns = self
            .columns
            .iter()
            .enumerate()
            .filter(|(i, c)| !reuse || (*i != 0 && !(integer_id && c.as_str() == "id")))
            .collect::<Vec<_>>();
        let sql = format!(
            "INSERT INTO {} ({}) VALUES ({})",
            self.table,
            columns
                .iter()
                .map(|(_, c)| format!("\"{c}\""))
                .collect::<Vec<_>>()
                .join(","),
            vec!["?"; columns.len()].join(",")
        );
        for row in &self.values {
            let values = columns
                .iter()
                .map(|(i, _)| &row[*i])
                .collect::<Vec<&Value>>();
            tx.execute(&sql, rusqlite::params_from_iter(values))?;
        }
        Ok(())
    }
}
fn visit_references(tx: &Transaction) -> Result<Vec<(String, String, String)>> {
    let mut stmt=tx.prepare("SELECT m.name,f.\"from\",f.on_delete FROM sqlite_master m,pragma_foreign_key_list(m.name) f WHERE m.type='table' AND f.\"table\"='visits' ORDER BY m.name,f.\"from\"")?;
    let rows = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<rusqlite::Result<_>>()?;
    Ok(rows)
}
impl Browser {
    /// Deletes settled trees. Trees no longer in History are left alone.
    pub fn delete_trees(
        &mut self,
        trees: &[TreeId],
    ) -> Result<Option<(DeleteId, Deleted, Landing)>> {
        let mut roots = vec![];
        for tree in trees {
            if !roots.contains(&tree.root())
                && self.connection.query_row(
                    "SELECT EXISTS(SELECT 1 FROM tree_state WHERE root_id=?1 AND place='history')",
                    [tree.root().to_string()],
                    |r| r.get::<_, bool>(0),
                )?
            {
                roots.push(tree.root());
            }
        }
        self.delete_heads(&roots, None)
    }
    /// Deletes `head` and every page opened from it, in any place. A root deletes its whole
    /// tree. When the current page is inside an open tree's branch, the window lands on the
    /// nearest ancestor left.
    pub fn delete_branch(&mut self, head: VisitId) -> Result<Option<(DeleteId, Deleted, Landing)>> {
        let forest =
            crate::forest::ForestProjection::from_snapshot(self.snapshot()?, self.tree_states()?)?;
        let Some(visit) = forest.visit(head) else {
            return Ok(None);
        };
        let open = forest
            .tree_of(head)
            .and_then(|t| forest.tree(t))
            .is_some_and(|t| t.place.is_tab());
        let landing = match visit.parent {
            Some(parent)
                if open
                    && forest
                        .current()
                        .is_some_and(|id| forest.path(id).contains(&head)) =>
            {
                Some(Landing::Restore(
                    forest
                        .path(parent)
                        .into_iter()
                        .rev()
                        .find(|id| !forest.is_settled(*id))
                        .context("No unsettled ancestor.")?,
                ))
            }
            _ => None,
        };
        self.delete_heads(&[head], landing)
    }
    fn delete_heads(
        &mut self,
        heads: &[VisitId],
        landing: Option<Landing>,
    ) -> Result<Option<(DeleteId, Deleted, Landing)>> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute_batch(
            "CREATE TEMP TABLE IF NOT EXISTS deleting(id TEXT PRIMARY KEY); DELETE FROM deleting;",
        )?;
        let mut kept = vec![];
        for head in heads {
            let parent: Option<Option<String>> = tx
                .query_row(
                    "SELECT parent_id FROM visits WHERE id=?1",
                    [head.to_string()],
                    |r| r.get(0),
                )
                .optional()?;
            let Some(parent) = parent else { continue };
            let parent: Option<VisitId> = parent.map(|p| p.parse()).transpose()?;
            kept.push((*head, parent));
            tx.execute("WITH RECURSIVE branch(id) AS (SELECT ?1 UNION ALL SELECT v.id FROM visits v JOIN branch b ON v.parent_id=b.id) INSERT OR IGNORE INTO deleting SELECT id FROM branch",[head.to_string()])?;
        }
        if kept.is_empty() {
            return Ok(None);
        }
        let heads = kept;
        let visits = tx
            .prepare("SELECT id FROM deleting")?
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?
            .into_iter()
            .map(|s| s.parse().map_err(anyhow::Error::from))
            .collect::<Result<_>>()?;
        let mut tables = vec![Rows::read(&tx, "visits", "id IN deleting")?];
        for (table, column, action) in visit_references(&tx)? {
            match action.as_str() {
                "CASCADE" => {
                    tables.push(Rows::read(&tx, &table, &format!("{column} IN deleting"))?)
                }
                "SET NULL" if table == "visit_events" && column == "from_visit_id" => {}
                "NO ACTION"
                    if matches!(
                        (table.as_str(), column.as_str()),
                        ("visits", "parent_id")
                            | ("tree_state", "root_id")
                            | ("tree_state", "last_visit_id")
                            | ("session", "current_id")
                    ) => {}
                _ => bail!("Uncovered visit reference {table}.{column}"),
            }
        }
        tables.push(Rows::read(&tx, "tree_state", "root_id IN deleting")?);
        let nulled=tx.prepare("SELECT id,from_visit_id FROM visit_events WHERE from_visit_id IN deleting AND visit_id NOT IN deleting")?.query_map([],|r|Ok((r.get(0)?,r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
        let roots: Vec<TreeId> = heads
            .iter()
            .filter(|(_, parent)| parent.is_none())
            .map(|(head, _)| TreeId(*head))
            .collect();
        let declined = roots
            .iter()
            .map(|t| format!("tree {}", t.root()))
            .filter(|key| {
                tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM declined_suggestions WHERE key=?1)",
                    [key],
                    |r| r.get::<_, bool>(0),
                )
                .unwrap_or(false)
            })
            .collect::<Vec<_>>();
        // A tree that keeps its root but loses its last page resumes at the nearest page
        // opened above the branch.
        let mut retargeted = vec![];
        for (head, parent) in &heads {
            let Some(parent) = parent else { continue };
            let last: Option<String> = tx.query_row("SELECT t.last_visit_id FROM tree_state t WHERE t.last_visit_id IN deleting AND t.root_id NOT IN deleting AND t.last_visit_id IN (WITH RECURSIVE branch(id) AS (SELECT ?1 UNION ALL SELECT v.id FROM visits v JOIN branch b ON v.parent_id=b.id) SELECT id FROM branch)",[head.to_string()],|r|r.get(0)).optional()?;
            let Some(last) = last else { continue };
            let keep: String = tx.query_row("WITH RECURSIVE up(id,parent_id,opened_at,depth) AS (SELECT id,parent_id,opened_at,0 FROM visits WHERE id=?1 UNION ALL SELECT v.id,v.parent_id,v.opened_at,u.depth+1 FROM visits v JOIN up u ON v.id=u.parent_id) SELECT id FROM up WHERE opened_at IS NOT NULL ORDER BY depth LIMIT 1",[parent.to_string()],|r|r.get(0)).optional()?.context("No opened page is left above this branch.")?;
            tx.execute(
                "UPDATE tree_state SET last_visit_id=?2 WHERE last_visit_id=?1",
                params![last, keep],
            )?;
            retargeted.push((last.parse()?, keep.parse()?));
        }
        // An open tree deleted whole leaves Tabs first, so the landing treats it as settled.
        tx.execute(
            "UPDATE tree_state SET place='history',position=NULL,pinned=0 WHERE root_id IN deleting AND place<>'history'",
            [],
        )?;
        let landing = match landing {
            Some(landing) => landing,
            None => land(&tx)?,
        };
        tx.execute(
            "UPDATE session SET current_id=NULL WHERE current_id IN deleting",
            [],
        )?;
        tx.execute("DELETE FROM tree_state WHERE root_id IN deleting", [])?;
        for key in &declined {
            tx.execute("DELETE FROM declined_suggestions WHERE key=?1", [key])?;
        }
        tx.execute("DELETE FROM visits WHERE id IN deleting", [])?;
        tx.commit()?;
        let id = DeleteId(ulid::Ulid::new());
        Ok(Some((
            id,
            Deleted {
                id,
                trees: roots,
                heads,
                visits,
                tables,
                nulled,
                declined,
                retargeted,
            },
            landing,
        )))
    }
    pub fn undo_delete(&mut self, id: DeleteId, deleted: &Deleted) -> Result<()> {
        if id != deleted.id {
            return Ok(());
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute_batch("PRAGMA defer_foreign_keys=ON")?;
        let exists = |id: VisitId| {
            tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM visits WHERE id=?1)",
                [id.to_string()],
                |r| r.get::<_, bool>(0),
            )
            .unwrap_or(true)
        };
        if deleted.trees.iter().any(|t| {
            tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM tree_state WHERE root_id=?1)",
                [t.root().to_string()],
                |r| r.get::<_, bool>(0),
            )
            .unwrap_or(false)
        }) || deleted
            .heads
            .iter()
            .any(|(head, parent)| exists(*head) || parent.is_some_and(|p| !exists(p)))
        {
            return Ok(());
        }
        for table in &deleted.tables {
            table.restore(&tx)?;
        }
        for (row, from) in &deleted.nulled {
            tx.execute(
                "UPDATE visit_events SET from_visit_id=?2 WHERE id=?1 AND from_visit_id IS NULL",
                params![row, from],
            )?;
        }
        for key in &deleted.declined {
            tx.execute(
                "INSERT OR IGNORE INTO declined_suggestions(key) VALUES(?1)",
                [key],
            )?;
        }
        for (last, keep) in &deleted.retargeted {
            tx.execute(
                "UPDATE tree_state SET last_visit_id=?1 WHERE last_visit_id=?2 AND root_id=(WITH RECURSIVE up(id,parent_id) AS (SELECT id,parent_id FROM visits WHERE id=?1 UNION ALL SELECT v.id,v.parent_id FROM visits v JOIN up u ON v.id=u.parent_id) SELECT id FROM up WHERE parent_id IS NULL)",
                params![last.to_string(), keep.to_string()],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
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
            16
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
    fn migration_15_keeps_reordered_tabs_and_removes_the_old_order_column() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("forest.sqlite3");
        let c = Connection::open(&path).unwrap();
        c.execute_batch(include_str!("../../tests/fixtures/schema-v14.sql"))
            .unwrap();
        let ids = (1..=6)
            .map(|i| ulid::Ulid::from(i as u128).to_string())
            .collect::<Vec<_>>();
        for (i, id) in ids.iter().enumerate() {
            c.execute("INSERT INTO visits(id,url,created_at,opened_at) VALUES(?1,'https://example.org',?2,1)",params![id,i]).unwrap();
            c.execute("INSERT INTO tree_state(root_id,last_visit_id,settled_at,pin_order) VALUES(?1,?1,?2,?3)",params![id,if i>=4 {Some(2)}else{None},match i {0|2=>Some(i+1),5=>Some(9),_=>None}]).unwrap();
        }
        c.execute_batch("ALTER TABLE tree_state ADD COLUMN tab_order INTEGER CHECK(tab_order >= 0);
            UPDATE tree_state SET tab_order=CASE root_id WHEN '00000000000000000000000001' THEN 3 WHEN '00000000000000000000000002' THEN 2 WHEN '00000000000000000000000003' THEN 1 WHEN '00000000000000000000000004' THEN 0 END;
            PRAGMA user_version=15;").unwrap();
        drop(c);
        let b = Browser::open(&path).unwrap();
        let states = b.tree_states().unwrap();
        assert_eq!(
            states
                .iter()
                .map(|s| s.root.root().to_string())
                .collect::<Vec<_>>(),
            [2, 0, 3, 1, 5, 4].map(|i| ids[i].clone())
        );
        assert_eq!(
            b.connection
                .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            16
        );
        assert!(!b
            .connection
            .prepare("SELECT 1 FROM pragma_table_info('tree_state') WHERE name='tab_order'")
            .unwrap()
            .exists([])
            .unwrap());
        assert!(!b
            .connection
            .prepare("PRAGMA foreign_key_check")
            .unwrap()
            .exists([])
            .unwrap());
        drop(b);
        assert_eq!(Browser::open(&path).unwrap().tree_states().unwrap(), states);
    }

    #[test]
    fn keyboard_steps_stop_at_group_edges_and_keep_pins() {
        let mut b = browser();
        let t = roots(&mut b, 6);
        for id in &t[..3] {
            b.place_tree(*id, Move::Pin(true)).unwrap();
        }
        let current = b.current_id().unwrap();
        for (id, next) in [(t[0], false), (t[2], true), (t[3], false), (t[5], true)] {
            assert_eq!(b.tree_step(id, next).unwrap(), None);
        }
        for (id, next, expected) in [
            (t[1], true, vec![t[0], t[2], t[1], t[3], t[4], t[5]]),
            (t[1], false, t.clone()),
            (t[4], false, vec![t[0], t[1], t[2], t[4], t[3], t[5]]),
            (t[4], true, t.clone()),
        ] {
            b.place_tree(id, b.tree_step(id, next).unwrap().unwrap())
                .unwrap();
            let states = b.tree_states().unwrap();
            assert_eq!(states.iter().map(|s| s.root).collect::<Vec<_>>(), expected);
            assert_eq!(
                states
                    .iter()
                    .map(|s| s.place().pinned())
                    .collect::<Vec<_>>(),
                [true, true, true, false, false, false]
            );
            assert_eq!(b.current_id().unwrap(), current);
        }
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
        for (left, right, expected) in [
            (None, None, [false, true]),
            (None, Some(false), [false, true]),
            (None, Some(true), [true, true]),
            (Some(false), None, [false, false]),
            (Some(false), Some(false), [false, false]),
            (Some(false), Some(true), [true, true]),
            (Some(true), None, [false, true]),
            (Some(true), Some(false), [false, true]),
            (Some(true), Some(true), [true, true]),
        ] {
            for (i, was) in [false, true].into_iter().enumerate() {
                assert_eq!(pin_after_drop(left, right, was), expected[i]);
            }
        }
    }
    #[test]
    fn visit_references_are_known_and_integer_keys_have_no_referrers() {
        let mut b = browser();
        let tx = b.connection.transaction().unwrap();
        assert_eq!(
            visit_references(&tx).unwrap(),
            vec![
                ("captures", "visit_id", "CASCADE"),
                ("downloads", "visit_id", "CASCADE"),
                ("origins", "visit_id", "CASCADE"),
                ("session", "current_id", "NO ACTION"),
                ("tree_state", "last_visit_id", "NO ACTION"),
                ("tree_state", "root_id", "NO ACTION"),
                ("visit_events", "from_visit_id", "SET NULL"),
                ("visit_events", "visit_id", "CASCADE"),
                ("visits", "parent_id", "NO ACTION")
            ]
            .into_iter()
            .map(|(a, b, c)| (a.into(), b.into(), c.into()))
            .collect::<Vec<_>>()
        );
        assert!(!tx.prepare("SELECT 1 FROM sqlite_master m,pragma_foreign_key_list(m.name) f WHERE m.type='table' AND (f.\"table\" IN ('captures','downloads','visit_events') OR (f.\"table\"='visits' AND f.\"to\"<>'id'))").unwrap().exists([]).unwrap());
    }
    fn all_rows(
        b: &mut Browser,
        normalize: bool,
    ) -> Vec<(String, Vec<Vec<rusqlite::types::Value>>)> {
        let names=b.connection.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'captures_fts_%' ORDER BY name").unwrap().query_map([],|r|r.get::<_,String>(0)).unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap();
        let tx = b.connection.transaction().unwrap();
        names
            .into_iter()
            .map(|name| {
                let mut rows = Rows::read(&tx, &name, "1").unwrap();
                if normalize {
                    let integer =
                        matches!(name.as_str(), "captures" | "downloads" | "visit_events");
                    rows.values = rows
                        .values
                        .into_iter()
                        .map(|r| {
                            r.into_iter()
                                .enumerate()
                                .filter(|(i, _)| *i != 0 && !(integer && rows.columns[*i] == "id"))
                                .map(|(_, v)| v)
                                .collect()
                        })
                        .collect();
                    rows.values.sort_by_key(|r| format!("{r:?}"));
                }
                (name, rows.values)
            })
            .collect()
    }
    fn enrich(b: &mut Browser, tree: TreeId, external: TreeId) {
        let id = tree.root().to_string();
        let child = ulid::Ulid::new().to_string();
        let queued = ulid::Ulid::new().to_string();
        b.connection.execute("INSERT INTO visits(id,parent_id,url,title,created_at,opened_at,scroll_y,branch_settled_at) VALUES(?1,?2,'https://child.test','Child',2,2,32,3)",params![child,id]).unwrap();
        b.connection.execute("INSERT INTO visits(id,parent_id,url,title,created_at,dismissed_at) VALUES(?1,?2,'https://waiting.test','Waiting',3,4)",params![queued,child]).unwrap();
        b.connection
            .execute(
                "UPDATE tree_state SET last_visit_id=?2 WHERE root_id=?1",
                params![id, child],
            )
            .unwrap();
        let other = external.root().to_string();
        b.connection
            .execute(
                "INSERT INTO origins VALUES(?1,'https://source.test',NULL,'link','excerpt')",
                [&id],
            )
            .unwrap();
        b.connection.execute("INSERT INTO captures(visit_id,document_url,title,body,captured_at,kind) VALUES(?1,'https://example.org','Captured','distinctiveword',1,'page')",[&id]).unwrap();
        b.connection.execute("INSERT INTO downloads(visit_id,path,size,saved_at) VALUES(?1,'/fixture/kept.pdf',12,1)",[&id]).unwrap();
        b.connection
            .execute(
                "INSERT INTO visit_events(visit_id,from_visit_id,kind,at) VALUES(?1,?2,'return',1)",
                params![other, id],
            )
            .unwrap();
        b.connection
            .execute(
                "INSERT INTO visit_events(visit_id,kind,at) VALUES(?1,'new',1)",
                [&id],
            )
            .unwrap();
        b.connection
            .execute(
                "INSERT INTO declined_suggestions VALUES(?1)",
                [format!("tree {id}")],
            )
            .unwrap();
        b.bookmark(&url::Url::parse("https://example.org").unwrap(), "Kept")
            .unwrap();
    }
    #[test]
    fn undo_restores_whole_database_and_reused_integer_keys() {
        for reuse in [false, true] {
            let mut b = browser();
            let t = roots(&mut b, 2);
            enrich(&mut b, t[1], t[0]);
            b.place_tree(t[1], Move::History).unwrap();
            let before = all_rows(&mut b, reuse);
            let (id, deleted, _) = b.delete_trees(&[t[0], t[1]]).unwrap().unwrap();
            b.undo_delete(DeleteId(ulid::Ulid::new()), &deleted)
                .unwrap();
            assert!(b.visit(t[1].root()).is_err());
            assert!(b.search("distinctiveword").unwrap().is_empty());
            assert_eq!(b.bookmarks().unwrap().len(), 1);
            assert!(b.delete_trees(&[t[1]]).unwrap().is_none());
            if reuse {
                let fresh = roots(&mut b, 1)[0];
                enrich(&mut b, fresh, t[0]);
                let with_new = all_rows(&mut b, true);
                b.undo_delete(id, &deleted).unwrap();
                let restored = all_rows(&mut b, true);
                let mut expected = with_new.clone();
                for (table, rows) in &mut expected {
                    let original = &before.iter().find(|(n, _)| n == table).unwrap().1;
                    if table == "sqlite_sequence" {
                        continue;
                    }
                    if table == "visit_events" {
                        for row in rows.iter_mut() {
                            if row[1] == rusqlite::types::Value::Null {
                                if let Some(old) = original
                                    .iter()
                                    .find(|old| old[0] == row[0] && old[2..] == row[2..])
                                {
                                    row[1] = old[1].clone();
                                }
                            }
                        }
                    }
                    for row in original {
                        if table == "captures_fts" || !rows.contains(row) {
                            rows.push(row.clone());
                        }
                    }
                    rows.sort_by_key(|r| format!("{r:?}"));
                }
                assert_eq!(expected, restored);
            } else {
                b.undo_delete(id, &deleted).unwrap();
                assert_eq!(before, all_rows(&mut b, false));
            }
            let restored = all_rows(&mut b, reuse);
            b.undo_delete(id, &deleted).unwrap();
            assert_eq!(restored, all_rows(&mut b, reuse));
            assert!(!b.search("distinctiveword").unwrap().is_empty());
            assert!(!b
                .connection
                .prepare("PRAGMA foreign_key_check")
                .unwrap()
                .exists([])
                .unwrap());
        }
    }
    fn page(b: &mut Browser, parent: VisitId) -> VisitId {
        let id: VisitId = ulid::Ulid::new().to_string().parse().unwrap();
        b.connection.execute("INSERT INTO visits(id,parent_id,url,title,created_at,opened_at) VALUES(?1,?2,'https://page.test','Page',2,2)",params![id.to_string(),parent.to_string()]).unwrap();
        id
    }
    fn without_session(
        rows: Vec<(String, Vec<Vec<rusqlite::types::Value>>)>,
    ) -> Vec<(String, Vec<Vec<rusqlite::types::Value>>)> {
        rows.into_iter().filter(|(t, _)| t != "session").collect()
    }
    #[test]
    fn deleting_a_branch_keeps_the_tree_and_undo_restores_it() {
        let mut b = browser();
        let t = roots(&mut b, 2);
        let root = t[0].root();
        let child = page(&mut b, root);
        let leaf = page(&mut b, child);
        let sibling = page(&mut b, root);
        b.connection.execute("INSERT INTO captures(visit_id,document_url,title,body,captured_at,kind) VALUES(?1,'https://page.test','Captured','branchword',1,'page')",[leaf.to_string()]).unwrap();
        b.connection
            .execute(
                "INSERT INTO visit_events(visit_id,from_visit_id,kind,at) VALUES(?1,?2,'return',1)",
                params![sibling.to_string(), leaf.to_string()],
            )
            .unwrap();
        b.connection
            .execute(
                "UPDATE tree_state SET last_visit_id=?2 WHERE root_id=?1",
                params![root.to_string(), leaf.to_string()],
            )
            .unwrap();
        b.connection
            .execute("UPDATE session SET current_id=?1", [leaf.to_string()])
            .unwrap();
        let before = all_rows(&mut b, false);
        let (id, deleted, landing) = b.delete_branch(child).unwrap().unwrap();
        assert_eq!(landing, Landing::Restore(root));
        assert!(deleted.trees().is_empty());
        assert_eq!(deleted.pages(), 2);
        assert!(deleted.holds(leaf) && !deleted.holds(sibling));
        assert!(b.visit(child).is_err() && b.visit(leaf).is_err());
        assert!(b.visit(sibling).is_ok());
        assert!(b.search("branchword").unwrap().is_empty());
        let state = b
            .tree_states()
            .unwrap()
            .into_iter()
            .find(|s| s.root.root() == root)
            .unwrap();
        assert_eq!(state.last_visit, root);
        assert!(state.place().is_tab());
        assert_eq!(b.snapshot().unwrap().current, None);
        b.undo_delete(id, &deleted).unwrap();
        assert_eq!(
            without_session(before),
            without_session(all_rows(&mut b, false))
        );
        assert!(!b
            .connection
            .prepare("PRAGMA foreign_key_check")
            .unwrap()
            .exists([])
            .unwrap());
    }
    #[test]
    fn a_branch_outside_the_current_page_keeps_the_window_where_it_is() {
        let mut b = browser();
        let root = roots(&mut b, 1)[0].root();
        let child = page(&mut b, root);
        let other = page(&mut b, root);
        b.connection
            .execute("UPDATE session SET current_id=?1", [other.to_string()])
            .unwrap();
        let (_, _, landing) = b.delete_branch(child).unwrap().unwrap();
        assert_eq!(landing, Landing::Unchanged);
        assert_eq!(b.snapshot().unwrap().current, Some(other));
    }
    #[test]
    fn deleting_an_open_root_removes_its_tab_and_undo_returns_it() {
        let mut b = browser();
        let t = roots(&mut b, 3);
        b.place_tree(t[1], Move::Pin(true)).unwrap();
        page(&mut b, t[1].root());
        b.connection
            .execute(
                "UPDATE session SET current_id=?1",
                [t[1].root().to_string()],
            )
            .unwrap();
        let before = all_rows(&mut b, false);
        let (id, deleted, landing) = b.delete_branch(t[1].root()).unwrap().unwrap();
        assert!(matches!(landing, Landing::Restore(v) if v != t[1].root()));
        assert_eq!(deleted.trees(), &[t[1]]);
        assert_eq!(deleted.pages(), 2);
        assert!(!b.tree_states().unwrap().iter().any(|s| s.root == t[1]));
        b.undo_delete(id, &deleted).unwrap();
        assert_eq!(
            without_session(before),
            without_session(all_rows(&mut b, false))
        );
        let state = b
            .tree_states()
            .unwrap()
            .into_iter()
            .find(|s| s.root == t[1])
            .unwrap();
        assert!(state.place().pinned());
    }
    #[test]
    fn undo_of_a_branch_waits_for_its_parent() {
        let mut b = browser();
        let tree = roots(&mut b, 1)[0];
        let child = page(&mut b, tree.root());
        let (id, branch, _) = b.delete_branch(child).unwrap().unwrap();
        let (tree_id, whole, _) = b.delete_branch(tree.root()).unwrap().unwrap();
        b.undo_delete(id, &branch).unwrap();
        assert!(b.visit(child).is_err());
        b.undo_delete(tree_id, &whole).unwrap();
        b.undo_delete(id, &branch).unwrap();
        assert!(b.visit(child).is_ok());
        assert!(b.delete_branch(child).unwrap().is_some());
        assert!(b.delete_branch(child).unwrap().is_none());
    }
    #[test]
    fn failed_undo_keeps_the_record_and_delete_is_durable() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("forest.sqlite3");
        let mut b = Browser::open(&path).unwrap();
        let tree = roots(&mut b, 1)[0];
        b.place_tree(tree, Move::History).unwrap();
        let (_, record, _) = b.delete_trees(&[tree]).unwrap().unwrap();
        b.connection.execute_batch("CREATE TRIGGER reject_restore BEFORE INSERT ON visits BEGIN SELECT RAISE(ABORT,'refused'); END").unwrap();
        assert!(b.undo_delete(record.id(), &record).is_err());
        assert!(b.visit(tree.root()).is_err());
        b.connection
            .execute_batch("DROP TRIGGER reject_restore")
            .unwrap();
        b.undo_delete(record.id(), &record).unwrap();
        b.delete_trees(&[tree]).unwrap();
        drop(b);
        assert!(Browser::open(&path).unwrap().visit(tree.root()).is_err());
    }
    #[test]
    fn a_deleted_tree_feeds_no_suggestion() {
        use crate::{
            forest::ForestProjection,
            suggest::{self, Inputs, Suggestion},
        };
        let mut b = browser();
        let tree = roots(&mut b, 1)[0];
        b.queue_link(
            tree.root(),
            url::Url::parse("https://waiting.test").unwrap(),
            super::super::SourceContext {
                page_url: url::Url::parse("https://example.org/0").unwrap(),
                citation_url: None,
                link_text: "Waiting".into(),
                excerpt: String::new(),
            },
        )
        .unwrap();
        b.connection
            .execute(
                "INSERT INTO visit_events(visit_id,kind,at) VALUES(?1,'return',1)",
                [tree.root().to_string()],
            )
            .unwrap();
        b.place_tree(tree, Move::History).unwrap();
        let rank = |b: &Browser| {
            let forest =
                ForestProjection::from_snapshot(b.snapshot().unwrap(), b.tree_states().unwrap())
                    .unwrap();
            let arrivals = b.arrivals(&forest, 0).unwrap();
            suggest::rank(
                &Inputs {
                    forest: &forest,
                    bookmarks: &[],
                    declined: &Default::default(),
                    arrivals: &arrivals,
                    chrome: &[],
                    left: None,
                },
                2 * suggest::DAY,
            )
        };
        assert!(rank(&b)
            .iter()
            .any(|s| matches!(s,Suggestion::Tree {tree:t,..} if *t==tree)));
        b.delete_trees(&[tree]).unwrap();
        assert!(rank(&b).is_empty());
    }
}
