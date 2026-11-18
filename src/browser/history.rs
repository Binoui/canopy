use super::{Browser, TreeId, VisitId};
use crate::{address::web_url, forest::ForestProjection, suggest::SuggestionKey};
use anyhow::Result;
use rusqlite::{params, OptionalExtension};
use std::collections::HashSet;
use url::Url;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ArrivalKind {
    New,
    Return,
}

#[derive(Clone, Debug)]
pub struct Arrival {
    pub visit: VisitId,
    pub tree: TreeId,
    pub url: Url,
    pub title: String,
    pub from: Option<VisitId>,
    pub parent: Option<VisitId>,
    pub kind: ArrivalKind,
    pub at: i64,
}

impl Browser {
    pub fn arrivals(&self, forest: &ForestProjection, since: i64) -> Result<Vec<Arrival>> {
        let mut stmt = self.connection.prepare("SELECT e.visit_id,e.from_visit_id,e.kind,e.at FROM visit_events e WHERE e.kind != 'reload' AND e.at >= ?1 ORDER BY e.at,e.id")?;
        let rows = stmt.query_map([since], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
            ))
        })?;
        let mut arrivals = Vec::new();
        for row in rows {
            let (id, from, kind, at) = row?;
            let visit: VisitId = id.parse()?;
            // The caller's forest can be one page older than the log.
            let (Some(page), Some(tree)) = (forest.visit(visit), forest.tree_of(visit)) else {
                continue;
            };
            arrivals.push(Arrival {
                visit,
                tree,
                url: web_url(page.url.as_str())?,
                title: page.title.clone(),
                from: from.map(|v| v.parse()).transpose()?,
                parent: page.parent,
                kind: if kind == "new" {
                    ArrivalKind::New
                } else {
                    ArrivalKind::Return
                },
                at,
            });
        }
        Ok(arrivals)
    }

    pub fn last_event_tree(&self, forest: &ForestProjection) -> Result<Option<TreeId>> {
        let id: Option<String> = self
            .connection
            .query_row(
                "SELECT visit_id FROM visit_events ORDER BY id DESC LIMIT 1",
                [],
                |r| r.get(0),
            )
            .optional()?;
        let Some(id) = id else { return Ok(None) };
        Ok(forest.tree_of(id.parse()?))
    }

    /// The most recently shown other tree that is still open. Event ids break ties
    /// when several selections share the same millisecond.
    pub fn previous_tree(&self) -> Result<Option<TreeId>> {
        let forest = ForestProjection::from_snapshot(self.snapshot()?, self.tree_states()?)?;
        let current = forest.current().and_then(|id| forest.tree_of(id));
        let mut stmt = self
            .connection
            .prepare("SELECT visit_id, from_visit_id FROM visit_events ORDER BY id DESC")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })?;
        for row in rows {
            let (destination, source) = row?;
            for id in std::iter::once(destination).chain(source) {
                let id: VisitId = id.parse()?;
                if let Some(tree) = forest.tree_of(id) {
                    if Some(tree) != current && forest.tabs().iter().any(|t| t.id == tree) {
                        return Ok(Some(tree));
                    }
                }
            }
        }
        Ok(None)
    }

    pub fn declined(&self) -> Result<HashSet<SuggestionKey>> {
        let mut stmt = self
            .connection
            .prepare("SELECT key FROM declined_suggestions")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        let mut keys = HashSet::new();
        for row in rows {
            if let Ok(key) = row?.parse() {
                keys.insert(key);
            }
        }
        Ok(keys)
    }

    pub fn decline(&mut self, key: &SuggestionKey) -> Result<()> {
        self.connection.execute(
            "INSERT OR IGNORE INTO declined_suggestions(key) VALUES(?1)",
            params![key.to_string()],
        )?;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct LogOrder(i64);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Shown {
    times: u32,
    last: LogOrder,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Showings {
    pages: std::collections::HashMap<VisitId, Shown>,
}

impl Showings {
    pub fn times(&self, id: VisitId) -> u32 {
        self.pages.get(&id).map_or(0, |s| s.times)
    }
    pub fn last_shown(&self, id: VisitId) -> Option<LogOrder> {
        self.pages.get(&id).map(|s| s.last)
    }
    fn eligible(&self, forest: &ForestProjection, id: VisitId) -> bool {
        forest.current().is_some_and(|current| {
            id != current
                && forest.tree_of(id) == forest.tree_of(current)
                && !forest.is_settled(id)
                && forest
                    .visit(id)
                    .is_some_and(|v| v.state == super::VisitState::Opened)
        })
    }
    pub fn recent(&self, forest: &ForestProjection) -> Vec<VisitId> {
        let mut pages: Vec<_> = self
            .pages
            .iter()
            .filter(|(id, _)| self.eligible(forest, **id))
            .collect();
        pages.sort_unstable_by_key(|(_, s)| std::cmp::Reverse(s.last));
        pages.into_iter().map(|(id, _)| *id).collect()
    }
    pub fn previous(&self, forest: &ForestProjection) -> Option<VisitId> {
        self.pages
            .iter()
            .filter(|(id, _)| self.eligible(forest, **id))
            .max_by_key(|(_, s)| s.last)
            .map(|(id, _)| *id)
    }
}
impl Browser {
    pub fn showings(&self) -> Result<Showings> {
        let mut stmt = self.connection.prepare(
            "WITH RECURSIVE open(id) AS (
            SELECT root_id FROM tree_state WHERE place='tab'
            UNION ALL SELECT v.id FROM visits v JOIN open o ON v.parent_id=o.id)
            SELECT e.visit_id, SUM(e.kind<>'reload'), MAX(e.id)
            FROM open o JOIN visit_events e ON e.visit_id=o.id GROUP BY e.visit_id",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, u32>(1)?,
                r.get::<_, i64>(2)?,
            ))
        })?;
        let mut pages = std::collections::HashMap::new();
        for row in rows {
            let (id, times, last) = row?;
            pages.insert(
                id.parse()?,
                Shown {
                    times,
                    last: LogOrder(last),
                },
            );
        }
        Ok(Showings { pages })
    }
}
