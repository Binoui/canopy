use super::{Browser, TreeId, VisitId};
use crate::{address::web_url, suggest::SuggestionKey};
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
    pub fn arrivals(&self, since: i64) -> Result<Vec<Arrival>> {
        let snapshot = self.snapshot()?;
        let forest = crate::forest::ForestProjection::from_snapshot(snapshot, self.tree_states()?)?;
        let mut stmt = self.connection.prepare("SELECT e.visit_id,e.from_visit_id,e.kind,e.at FROM visit_events e WHERE e.kind != 'reload' AND e.at >= ?1 ORDER BY e.at,e.id")?;
        let rows = stmt.query_map([since], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
            ))
        })?;
        rows.map(|r| {
            let (id, from, kind, at) = r?;
            let visit: VisitId = id.parse()?;
            let page = forest
                .visit(visit)
                .ok_or_else(|| anyhow::anyhow!("Missing arrival page."))?;
            Ok(Arrival {
                visit,
                tree: forest.tree_of(visit).unwrap(),
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
            })
        })
        .collect()
    }

    pub fn last_event_tree(&self) -> Result<Option<TreeId>> {
        let id: Option<String> = self
            .connection
            .query_row(
                "SELECT visit_id FROM visit_events ORDER BY at DESC,id DESC LIMIT 1",
                [],
                |r| r.get(0),
            )
            .optional()?;
        let Some(id) = id else { return Ok(None) };
        let forest =
            crate::forest::ForestProjection::from_snapshot(self.snapshot()?, self.tree_states()?)?;
        Ok(forest.tree_of(id.parse()?))
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
