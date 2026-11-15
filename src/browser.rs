mod bookmarks;
mod history;
pub use bookmarks::{Bookmark, BookmarkId, BookmarkToggle, ImportedBookmark};
pub use history::{Arrival, ArrivalKind};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension, Row};
use ulid::Ulid;
use url::Url;

use crate::address::web_url;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct VisitId(Ulid);

impl std::fmt::Display for VisitId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl std::str::FromStr for VisitId {
    type Err = ulid::DecodeError;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        s.parse().map(Self)
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct TreeId(pub(crate) VisitId);

impl TreeId {
    pub fn root(self) -> VisitId {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NavigationTarget {
    Root,
    ChildOf(VisitId),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VisitPreview {
    pub title: String,
    pub snippet: String,
}

/// A page known from another browser's history.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImportedPage {
    pub url: Url,
    pub title: String,
    pub visits: usize,
    pub typed: usize,
    /// Milliseconds since the Unix epoch.
    pub last_visit: Option<i64>,
}

/// Whether a tree has a tab. A settled tree keeps every page and reopens when one is visited.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Lifecycle {
    Open,
    /// `since` is in ms and orders the settled trees, newest first.
    Settled {
        since: i64,
    },
}

impl Lifecycle {
    pub fn is_open(self) -> bool {
        matches!(self, Self::Open)
    }
}

/// Where the window goes after a tree is settled.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Landing {
    /// The current page is in an open tree, or there is none.
    Unchanged,
    /// Load this page of the open tree visited last. The current page stays in its settled
    /// tree until that load commits.
    Restore(VisitId),
    /// No open tree is left. The current page is already cleared, so the caller must stop
    /// showing it.
    Start,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct PinOrder(std::num::NonZeroI64);

impl PinOrder {
    fn from_sql(value: i64) -> Result<Self> {
        anyhow::ensure!(value > 0, "Invalid pin order.");
        Ok(Self(std::num::NonZeroI64::new(value).unwrap()))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TreeState {
    pub pin: Option<PinOrder>,
    pub last_event_at: Option<i64>,
    pub root: TreeId,
    pub last_visit: VisitId,
    /// The name the user gave the tree. Without one, the tree is named after its pages.
    pub name: Option<String>,
    pub lifecycle: Lifecycle,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ThemeMode {
    System,
    Light,
    Dark,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VisitState {
    Queued,
    Opened,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceContext {
    pub page_url: Url,
    pub citation_url: Option<Url>,
    pub link_text: String,
    pub excerpt: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CaptureKind {
    Page,
    Selection,
}

impl CaptureKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Page => "page",
            Self::Selection => "selection",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CapturePayload {
    pub document_url: Url,
    pub citation_url: Option<Url>,
    pub title: String,
    pub text: String,
    pub kind: CaptureKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Capture {
    pub id: i64,
    pub visit: VisitId,
    pub document_url: Url,
    pub citation_url: Option<Url>,
    pub title: String,
    pub text: String,
    pub captured_at: i64,
    pub kind: CaptureKind,
    pub source: Option<SourceContext>,
}

/// A file saved from a page. The file stays where it was saved; Canopy only remembers
/// where that is, so it can be moved or deleted behind its back.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Download {
    pub id: i64,
    pub visit: VisitId,
    pub path: PathBuf,
    /// Bytes on disk when the file was saved.
    pub size: u64,
    /// Where the file came from, when that is a web address.
    pub source: Option<Url>,
    pub saved_at: i64,
}

impl Download {
    pub fn name(&self) -> String {
        self.path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchHit {
    pub visit: VisitId,
    pub capture_id: Option<i64>,
    pub title: String,
    pub excerpt: String,
    pub captured_at: Option<i64>,
    pub source: Option<SourceContext>,
    pub state: VisitState,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Visit {
    pub id: VisitId,
    pub parent: Option<VisitId>,
    pub url: Url,
    pub title: String,
    pub created_at: i64,
    pub opened_at: Option<i64>,
    pub state: VisitState,
    pub source: Option<SourceContext>,
}

impl Visit {
    pub fn label(&self) -> &str {
        if self.title.trim().is_empty() {
            self.url.host_str().unwrap_or(self.url.as_str())
        } else {
            &self.title
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Snapshot {
    pub visits: Vec<Visit>,
    pub current: Option<VisitId>,
}

impl Snapshot {
    pub fn current_visit(&self) -> Option<&Visit> {
        self.current
            .and_then(|id| self.visits.iter().find(|visit| visit.id == id))
    }

    pub fn breadcrumb(&self) -> Vec<&Visit> {
        let mut result = Vec::new();
        let mut cursor = self.current;
        while let Some(id) = cursor {
            let Some(visit) = self.visits.iter().find(|visit| visit.id == id) else {
                break;
            };
            result.push(visit);
            cursor = visit.parent;
        }
        result.reverse();
        result
    }
}

mod navigation;
pub use navigation::{Document, LoadTicket, Navigation, ViewId, Visibility};
use navigation::{Intent, Pending};

const MAX_TREE_NAME: usize = 80;

pub struct Browser {
    connection: Connection,
}

fn migrate_pins(connection: &Connection) -> Result<()> {
    connection.execute_batch(
        "ALTER TABLE tree_state ADD COLUMN pin_order INTEGER CHECK(pin_order > 0);
                 CREATE UNIQUE INDEX tree_state_pin_order ON tree_state(pin_order)
                    WHERE pin_order IS NOT NULL;
                 PRAGMA user_version = 9;",
    )?;
    Ok(())
}

impl Browser {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        Self::with_connection(Connection::open(path)?)
    }

    fn with_connection(mut connection: Connection) -> Result<Self> {
        connection.busy_timeout(std::time::Duration::from_secs(2))?;
        // `downloads` is created here and not in a numbered step below: it changes no
        // existing table, and a Canopy from before it simply never reads it.
        connection.execute_batch(
            "PRAGMA foreign_keys = ON;
             PRAGMA journal_mode = WAL;
             CREATE TABLE IF NOT EXISTS visits (
                 id TEXT PRIMARY KEY NOT NULL,
                 parent_id TEXT REFERENCES visits(id),
                 url TEXT NOT NULL,
                 title TEXT NOT NULL DEFAULT '',
                 created_at INTEGER NOT NULL
             );
             CREATE INDEX IF NOT EXISTS visits_parent ON visits(parent_id);
             CREATE TABLE IF NOT EXISTS downloads (
                 id INTEGER PRIMARY KEY,
                 visit_id TEXT NOT NULL REFERENCES visits(id) ON DELETE CASCADE,
                 path TEXT NOT NULL,
                 size INTEGER NOT NULL,
                 source_url TEXT,
                 saved_at INTEGER NOT NULL
             );
             CREATE INDEX IF NOT EXISTS downloads_visit ON downloads(visit_id, id);
             CREATE TABLE IF NOT EXISTS session (
                 singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
                 current_id TEXT REFERENCES visits(id)
             );
             INSERT OR IGNORE INTO session(singleton, current_id) VALUES(1, NULL);",
        )?;
        let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version > 11 {
            bail!("This database comes from a newer version of Canopy.");
        }
        if version == 0 {
            let transaction = connection.transaction()?;
            transaction.execute_batch(
                "ALTER TABLE visits ADD COLUMN opened_at INTEGER;
                 UPDATE visits SET opened_at = created_at;
                 ALTER TABLE session ADD COLUMN capture_enabled INTEGER NOT NULL DEFAULT 1;
                 CREATE TABLE origins (
                     visit_id TEXT PRIMARY KEY REFERENCES visits(id) ON DELETE CASCADE,
                     source_page_url TEXT NOT NULL,
                     source_citation_url TEXT,
                     source_link_text TEXT NOT NULL,
                     source_excerpt TEXT NOT NULL
                 );
                 CREATE TABLE captures (
                     id INTEGER PRIMARY KEY,
                     visit_id TEXT NOT NULL REFERENCES visits(id) ON DELETE CASCADE,
                     document_url TEXT NOT NULL,
                     citation_url TEXT,
                     title TEXT NOT NULL,
                     body TEXT NOT NULL,
                     captured_at INTEGER NOT NULL,
                     kind TEXT NOT NULL CHECK(kind IN ('page', 'selection')),
                     source_page_url TEXT,
                     source_citation_url TEXT,
                     source_link_text TEXT,
                     source_excerpt TEXT
                 );
                 CREATE INDEX captures_visit ON captures(visit_id, id DESC);
                 CREATE VIRTUAL TABLE captures_fts USING fts5(
                     title, body, document_url, citation_url, source_page_url,
                     source_link_text, source_excerpt,
                     content='captures', content_rowid='id'
                 );
                 CREATE TRIGGER captures_insert AFTER INSERT ON captures BEGIN
                     INSERT INTO captures_fts(rowid, title, body, document_url, citation_url,
                         source_page_url, source_link_text, source_excerpt)
                     VALUES(new.id, new.title, new.body, new.document_url, new.citation_url,
                         new.source_page_url, new.source_link_text, new.source_excerpt);
                 END;
                 CREATE TRIGGER captures_delete AFTER DELETE ON captures BEGIN
                     INSERT INTO captures_fts(captures_fts, rowid, title, body, document_url,
                         citation_url, source_page_url, source_link_text, source_excerpt)
                     VALUES('delete', old.id, old.title, old.body, old.document_url,
                         old.citation_url, old.source_page_url, old.source_link_text, old.source_excerpt);
                 END;
                 PRAGMA user_version = 1;",
            )?;
            transaction.commit()?;
        }
        if version < 2 {
            let transaction = connection.transaction()?;
            transaction.execute_batch(
                "CREATE TABLE tree_state (
                    root_id TEXT PRIMARY KEY NOT NULL REFERENCES visits(id),
                    last_visit_id TEXT NOT NULL REFERENCES visits(id)
                 );
                 ALTER TABLE session ADD COLUMN theme_mode TEXT NOT NULL DEFAULT 'system'
                    CHECK(theme_mode IN ('system', 'light', 'dark'));
                 INSERT INTO tree_state(root_id, last_visit_id)
                    SELECT id, id FROM visits WHERE parent_id IS NULL;
                 WITH RECURSIVE ancestry(id, parent_id) AS (
                    SELECT id, parent_id FROM visits
                    WHERE id = (SELECT current_id FROM session WHERE singleton = 1)
                    UNION
                    SELECT v.id, v.parent_id FROM visits v JOIN ancestry a ON v.id=a.parent_id
                 )
                 UPDATE tree_state SET last_visit_id =
                    (SELECT current_id FROM session WHERE singleton = 1)
                    WHERE root_id IN (SELECT id FROM ancestry WHERE parent_id IS NULL);
                 PRAGMA user_version = 2;",
            )?;
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
            transaction.commit()?;
        }
        if version < 3 {
            // Dark became the default theme. A forest still on the old `system` default starts
            // dark too; System stays one click away in Settings.
            let transaction = connection.transaction()?;
            transaction.execute_batch(
                "UPDATE session SET theme_mode = 'dark' WHERE theme_mode = 'system';
                 PRAGMA user_version = 3;",
            )?;
            transaction.commit()?;
        }
        if version < 4 {
            // Pages and bookmarks imported from another browser. They stay out of the forest:
            // search and the new-tree page read them, and opening one starts a visit.
            let transaction = connection.transaction()?;
            transaction.execute_batch(
                "CREATE TABLE imported_pages (
                    source TEXT NOT NULL,
                    url TEXT NOT NULL,
                    title TEXT NOT NULL,
                    visits INTEGER NOT NULL,
                    typed INTEGER NOT NULL,
                    last_visit INTEGER,
                    folder TEXT,
                    PRIMARY KEY(source, url)
                 );
                 PRAGMA user_version = 4;",
            )?;
            transaction.commit()?;
        }
        if version < 5 {
            // A name the user chose for a tree. NULL keeps the automatic name.
            let transaction = connection.transaction()?;
            transaction.execute_batch(
                "ALTER TABLE tree_state ADD COLUMN name TEXT;
                 PRAGMA user_version = 5;",
            )?;
            transaction.commit()?;
        }
        if version < 6 {
            // One row each time a page becomes the current page: a new visit, a return to an
            // earlier one, or a reload. `from_visit_id` is the page that was current before, so
            // the log keeps revisits, tree switches and time between pages, which the forest
            // alone cannot. Opened visits from before the log count as one arrival each.
            let transaction = connection.transaction()?;
            transaction.execute_batch(
                "CREATE TABLE visit_events (
                    id INTEGER PRIMARY KEY,
                    visit_id TEXT NOT NULL REFERENCES visits(id) ON DELETE CASCADE,
                    from_visit_id TEXT REFERENCES visits(id) ON DELETE SET NULL,
                    kind TEXT NOT NULL CHECK(kind IN ('new', 'return', 'reload')),
                    at INTEGER NOT NULL
                 );
                 CREATE INDEX visit_events_visit ON visit_events(visit_id, at);
                 CREATE INDEX visit_events_at ON visit_events(at);
                 INSERT INTO visit_events(visit_id, from_visit_id, kind, at)
                    SELECT id, parent_id, 'new', opened_at FROM visits
                    WHERE opened_at IS NOT NULL ORDER BY opened_at, id;
                 PRAGMA user_version = 6;",
            )?;
            transaction.commit()?;
        }
        if version < 7 {
            // Where the user left each page, in CSS pixels from the top, so a page reloaded
            // from the tree opens where they stopped reading. NULL until the page is scrolled.
            let transaction = connection.transaction()?;
            transaction.execute_batch(
                "ALTER TABLE visits ADD COLUMN scroll_y INTEGER;
                 PRAGMA user_version = 7;",
            )?;
            transaction.commit()?;
        }
        if version < 8 {
            let transaction = connection.transaction()?;
            transaction.execute_batch(
                "ALTER TABLE tree_state ADD COLUMN settled_at INTEGER CHECK(settled_at > 0);
                 PRAGMA user_version = 8;",
            )?;
            transaction.commit()?;
        }
        if version < 9 {
            let transaction = connection.transaction()?;
            migrate_pins(&transaction)?;
            transaction.commit()?;
        }
        if version < 10 {
            let tx = connection.transaction()?;
            bookmarks::migrate(&tx)?;
            tx.commit()?;
        }
        if version < 11 {
            let tx = connection.transaction()?;
            tx.execute_batch("CREATE TABLE declined_suggestions(key TEXT PRIMARY KEY NOT NULL); PRAGMA user_version=11;")?;
            tx.commit()?;
        }
        Ok(Self { connection })
    }

    pub fn replace_imported(&mut self, source: &str, pages: &[ImportedPage]) -> Result<usize> {
        self.import_pages(source, pages, &[])?;
        Ok(pages.len())
    }

    /// Imported pages whose title or URL contains `query`, typed and visited most first.
    pub fn search_imported(&self, query: &str, limit: usize) -> Result<Vec<ImportedPage>> {
        let query = query.trim();
        if query.is_empty() {
            return Ok(Vec::new());
        }
        let literal = query
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_");
        let mut statement = self.connection.prepare(
            r"SELECT url, MAX(title), SUM(visits), SUM(typed), MAX(last_visit)
              FROM imported_pages
              WHERE title LIKE ?1 ESCAPE '\' OR url LIKE ?1 ESCAPE '\'
              GROUP BY url
              ORDER BY SUM(typed) DESC, SUM(visits) DESC, MAX(last_visit) DESC
              LIMIT ?2",
        )?;
        let rows =
            statement.query_map(params![format!("%{literal}%"), limit as i64], imported_row)?;
        let mut pages = Vec::new();
        for row in rows {
            pages.extend(row?);
        }
        Ok(pages)
    }

    /// The most visited imported page of each of the busiest hosts.
    pub fn chrome_favorites(&self, limit: usize) -> Result<Vec<ImportedPage>> {
        let mut statement = self.connection.prepare(
            "SELECT url, MAX(title), SUM(visits), SUM(typed), MAX(last_visit)
             FROM imported_pages WHERE visits > 0 GROUP BY url
             ORDER BY SUM(typed) DESC, SUM(visits) DESC, url",
        )?;
        let mut hosts = HashSet::new();
        let mut favorites = Vec::new();
        let rows = statement.query_map([], imported_row)?;
        for row in rows {
            let Some(page) = row? else { continue };
            if hosts.insert(crate::suggest::site(&page.url).to_owned()) {
                favorites.push(page);
                if favorites.len() == limit {
                    break;
                }
            }
        }
        Ok(favorites)
    }

    pub fn snapshot(&self) -> Result<Snapshot> {
        let mut statement = self.connection.prepare(
            "SELECT v.id, v.parent_id, v.url, v.title, v.created_at, v.opened_at,
                o.source_page_url, o.source_citation_url, o.source_link_text, o.source_excerpt
             FROM visits v LEFT JOIN origins o ON o.visit_id=v.id ORDER BY v.created_at, v.rowid",
        )?;
        let mut rows = statement.query([])?;
        let mut visits = Vec::new();
        while let Some(row) = rows.next()? {
            let opened_at: Option<i64> = row.get(5)?;
            visits.push(Visit {
                id: row.get::<_, String>(0)?.parse()?,
                parent: row
                    .get::<_, Option<String>>(1)?
                    .map(|id| id.parse())
                    .transpose()?,
                url: web_url(&row.get::<_, String>(2)?)?,
                title: row.get(3)?,
                created_at: row.get(4)?,
                opened_at,
                state: if opened_at.is_some() {
                    VisitState::Opened
                } else {
                    VisitState::Queued
                },
                source: source_from_row(row, 6)?,
            });
        }
        let current = self.current_id()?;
        Ok(Snapshot { visits, current })
    }

    pub fn preview(&self, id: VisitId) -> Result<VisitPreview> {
        self.connection
            .query_row(
                "SELECT CASE WHEN trim(v.title)='' THEN v.url ELSE v.title END,
                COALESCE(NULLIF(substr(o.source_excerpt, 1, 320), ''),
                    (SELECT substr(c.body, 1, 320) FROM captures c
                     WHERE c.visit_id=v.id ORDER BY c.id DESC LIMIT 1), '')
             FROM visits v LEFT JOIN origins o ON o.visit_id=v.id WHERE v.id=?1",
                [id.to_string()],
                |row| {
                    Ok(VisitPreview {
                        title: row.get(0)?,
                        snippet: row.get(1)?,
                    })
                },
            )
            .context("This visit no longer exists.")
            .map(|mut preview| {
                let snippet = preview.snippet.trim();
                preview.snippet = snippet
                    .strip_prefix(preview.title.trim())
                    .unwrap_or(snippet)
                    .trim_start_matches(|c: char| {
                        c.is_whitespace() || matches!(c, '|' | ':' | '–' | '-')
                    })
                    .to_owned();
                preview
            })
    }

    pub fn tree_states(&self) -> Result<Vec<TreeState>> {
        let mut statement = self.connection.prepare(
            "WITH RECURSIVE members(root_id, visit_id) AS (
    SELECT id, id FROM visits WHERE parent_id IS NULL
    UNION ALL
    SELECT m.root_id, v.id FROM visits v JOIN members m ON v.parent_id=m.visit_id
), activity(root_id, latest_event_at) AS (
    SELECT m.root_id, MAX(e.at)
    FROM members m LEFT JOIN visit_events e ON e.visit_id=m.visit_id
    GROUP BY m.root_id
)
SELECT t.root_id, t.last_visit_id, t.name, t.settled_at, t.pin_order,
       a.latest_event_at
FROM tree_state t
JOIN visits r ON r.id=t.root_id
LEFT JOIN activity a ON a.root_id=t.root_id
ORDER BY r.created_at, r.rowid",
        )?;
        let mut rows = statement.query([])?;
        let mut states = Vec::new();
        while let Some(row) = rows.next()? {
            states.push(TreeState {
                pin: row
                    .get::<_, Option<i64>>(4)?
                    .map(PinOrder::from_sql)
                    .transpose()?,
                last_event_at: row.get(5)?,
                root: TreeId(row.get::<_, String>(0)?.parse()?),
                last_visit: row.get::<_, String>(1)?.parse()?,
                name: row.get(2)?,
                lifecycle: row
                    .get::<_, Option<i64>>(3)?
                    .map_or(Lifecycle::Open, |since| Lifecycle::Settled { since }),
            });
        }
        Ok(states)
    }

    pub fn set_pinned(&mut self, tree: TreeId, pinned: bool) -> Result<()> {
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let order: Option<Option<i64>> = tx
            .query_row(
                "SELECT pin_order FROM tree_state WHERE root_id=?1",
                [tree.root().to_string()],
                |r| r.get(0),
            )
            .optional()?;
        if order.is_none() || (pinned && order.flatten().is_some()) {
            return Ok(());
        }
        if pinned {
            let next: i64 = tx.query_row(
                "SELECT COALESCE(MAX(pin_order), 0) FROM tree_state",
                [],
                |r| r.get(0),
            )?;
            let next = next.checked_add(1).context("Too many pinned trees.")?;
            tx.execute(
                "UPDATE tree_state SET pin_order=?2 WHERE root_id=?1 AND pin_order IS NULL",
                params![tree.root().to_string(), next],
            )?;
        } else {
            tx.execute(
                "UPDATE tree_state SET pin_order=NULL WHERE root_id=?1",
                [tree.root().to_string()],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Takes `tree` out of the tabs and says where the window goes. Settling a settled tree
    /// changes nothing and gives the landing again, so a failed landing can be retried.
    pub fn settle_tree(&mut self, tree: TreeId) -> Result<Landing> {
        let tx = self.connection.transaction()?;
        let root = tree.0.to_string();
        if tx.execute(
            "UPDATE tree_state SET settled_at=COALESCE(settled_at, ?2) WHERE root_id=?1",
            params![root, now_ms()?],
        )? == 0
        {
            bail!("This tree no longer exists.");
        }
        // A window already resting in a settled tree, after a landing that failed or has not
        // committed, gets a landing again whichever tree is settled now.
        let stranded: Option<bool> = tx.query_row(
            "WITH RECURSIVE ancestry(id, parent_id) AS (
                SELECT id, parent_id FROM visits WHERE id=(SELECT current_id FROM session WHERE singleton=1)
                UNION
                SELECT v.id, v.parent_id FROM visits v JOIN ancestry a ON v.id=a.parent_id
             ) SELECT t.settled_at IS NOT NULL FROM ancestry a JOIN tree_state t ON t.root_id=a.id
               WHERE a.parent_id IS NULL",
            [], |row| row.get(0),
        ).optional()?;
        if stranded != Some(true) {
            tx.commit()?;
            return Ok(Landing::Unchanged);
        }
        let next: Option<String> = tx.query_row(
            "SELECT t.last_visit_id FROM tree_state t JOIN visits r ON r.id=t.root_id
             WHERE t.settled_at IS NULL
             ORDER BY (SELECT MAX(e.id) FROM visit_events e WHERE e.visit_id=t.last_visit_id) IS NULL,
                      (SELECT MAX(e.id) FROM visit_events e WHERE e.visit_id=t.last_visit_id) DESC,
                      r.created_at DESC, r.rowid DESC LIMIT 1",
            [], |row| row.get(0),
        ).optional()?;
        if let Some(next) = next {
            let visit = next.parse()?;
            tx.commit()?;
            Ok(Landing::Restore(visit))
        } else {
            tx.execute("UPDATE session SET current_id=NULL WHERE singleton=1", [])?;
            tx.commit()?;
            Ok(Landing::Start)
        }
    }

    /// Gives `tree` a name that stays whatever its pages become. An empty name goes back to
    /// naming the tree after its pages.
    pub fn rename_tree(&mut self, tree: TreeId, name: &str) -> Result<()> {
        let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
        let name: String = name.chars().take(MAX_TREE_NAME).collect();
        let renamed = self.connection.execute(
            "UPDATE tree_state SET name=?1 WHERE root_id=?2",
            params![(!name.is_empty()).then_some(name), tree.0.to_string()],
        )?;
        if renamed == 0 {
            bail!("This tree no longer exists.");
        }
        Ok(())
    }

    pub fn tree_target(&self, tree: TreeId) -> Result<VisitId> {
        let id: String = self
            .connection
            .query_row(
                "SELECT last_visit_id FROM tree_state WHERE root_id=?1",
                [tree.0.to_string()],
                |row| row.get(0),
            )
            .context("This tree no longer exists.")?;
        Ok(id.parse()?)
    }

    pub fn search(&self, query: &str) -> Result<Vec<SearchHit>> {
        let query = query.trim();
        if query.is_empty() {
            return Ok(Vec::new());
        }
        let mut hits = Vec::new();
        let mut captured_visits = HashSet::new();
        if query.chars().any(char::is_alphanumeric) {
            let phrase = query
                .strip_prefix('"')
                .and_then(|q| q.strip_suffix('"'))
                .unwrap_or(query);
            let literal = format!("\"{}\"", phrase.replace('"', "\"\""));
            let mut statement = self.connection.prepare(
                "SELECT c.visit_id, c.id, c.title, snippet(captures_fts, 1, '', '', '…', 28),
                    c.captured_at, c.source_page_url, c.source_citation_url,
                    c.source_link_text, c.source_excerpt
                 FROM captures_fts JOIN captures c ON c.id=captures_fts.rowid
                 WHERE captures_fts MATCH ?1 ORDER BY bm25(captures_fts), c.id DESC LIMIT 100",
            )?;
            let mut rows = statement.query([literal])?;
            while let Some(row) = rows.next()? {
                let visit = row.get::<_, String>(0)?.parse()?;
                captured_visits.insert(visit);
                hits.push(SearchHit {
                    visit,
                    capture_id: Some(row.get(1)?),
                    title: row.get(2)?,
                    excerpt: row.get(3)?,
                    captured_at: Some(row.get(4)?),
                    source: source_from_row(row, 5)?,
                    state: VisitState::Opened,
                });
            }
        }
        let literal = query
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_");
        let pattern = format!("%{literal}%");
        let mut statement = self.connection.prepare(
            r"SELECT v.id FROM visits v LEFT JOIN origins o ON o.visit_id=v.id
              WHERE v.title LIKE ?1 ESCAPE '\' OR v.url LIKE ?1 ESCAPE '\'
                 OR o.source_page_url LIKE ?1 ESCAPE '\' OR o.source_excerpt LIKE ?1 ESCAPE '\'
                 OR o.source_link_text LIKE ?1 ESCAPE '\'
              ORDER BY v.created_at DESC, v.rowid DESC LIMIT 100",
        )?;
        let ids = statement.query_map([pattern], |row| row.get::<_, String>(0))?;
        for id in ids {
            let id = id?.parse()?;
            if captured_visits.contains(&id) {
                continue;
            }
            let visit = self.visit(id)?;
            hits.push(SearchHit {
                visit: id,
                capture_id: None,
                title: visit.label().to_owned(),
                excerpt: visit
                    .source
                    .as_ref()
                    .map(|source| source.excerpt.clone())
                    .unwrap_or_default(),
                captured_at: None,
                source: visit.source,
                state: visit.state,
            });
        }
        Ok(hits)
    }

    fn current_id(&self) -> Result<Option<VisitId>> {
        let id: Option<String> = self.connection.query_row(
            "SELECT current_id FROM session WHERE singleton = 1",
            [],
            |row| row.get(0),
        )?;
        id.map(|id| id.parse().map_err(Into::into)).transpose()
    }

    pub fn visit(&self, id: VisitId) -> Result<Visit> {
        self.snapshot()?
            .visits
            .into_iter()
            .find(|visit| visit.id == id)
            .context("This visit no longer exists.")
    }

    pub fn navigate(
        &mut self,
        navigation: &mut Navigation,
        url: Url,
        target: NavigationTarget,
    ) -> Result<Url> {
        web_url(url.as_str())?;
        let parent = match target {
            NavigationTarget::Root => None,
            NavigationTarget::ChildOf(parent) => {
                let opened: bool = self
                    .connection
                    .query_row(
                        "SELECT opened_at IS NOT NULL FROM visits WHERE id=?1",
                        [parent.to_string()],
                        |row| row.get(0),
                    )
                    .context("The navigation parent does not exist.")?;
                if !opened {
                    bail!("The navigation parent has not been opened.");
                }
                Some(parent)
            }
        };
        navigation.request(
            Intent::New {
                parent,
                source: None,
            },
            url.clone(),
        );
        Ok(url)
    }

    /// Plants each URL as a new tree before its page loads, all of them or none.
    /// Restoring a root loads it.
    pub fn add_roots(&mut self, urls: &[Url]) -> Result<Vec<VisitId>> {
        for url in urls {
            web_url(url.as_str())?;
        }
        let transaction = self.connection.transaction()?;
        let mut roots = Vec::with_capacity(urls.len());
        for url in urls {
            let id = VisitId(Ulid::new());
            transaction.execute(
                "INSERT INTO visits(id, parent_id, url, title, created_at, opened_at) VALUES (?1, NULL, ?2, '', ?3, ?3)",
                params![id.to_string(), url.as_str(), now_ms()?],
            )?;
            transaction.execute(
                "INSERT INTO tree_state(root_id, last_visit_id) VALUES (?1, ?1)",
                [id.to_string()],
            )?;
            roots.push(id);
        }
        transaction.commit()?;
        Ok(roots)
    }

    pub fn restore(&mut self, navigation: &mut Navigation, id: VisitId) -> Result<Url> {
        let visit = self.visit(id)?;
        navigation.request(Intent::Restore { visit: id }, visit.url.clone());
        Ok(visit.url)
    }

    pub fn back(&mut self, navigation: &mut Navigation) -> Result<Option<Url>> {
        let Some(current) = navigation
            .current
            .as_ref()
            .map(|d| d.visit)
            .or(self.current_id()?)
        else {
            return Ok(None);
        };
        let Some(parent) = self.visit(current)?.parent else {
            return Ok(None);
        };
        self.restore(navigation, parent).map(Some)
    }

    /// The visit a requested navigation returns to, before its load starts.
    pub fn requested_restore(&self, navigation: &Navigation) -> Option<VisitId> {
        match &navigation.pending {
            Pending::Requested {
                intent: Intent::Restore { visit },
                ..
            } => Some(*visit),
            _ => None,
        }
    }

    fn set_scroll(&mut self, visit: VisitId, y: i64) -> Result<()> {
        self.connection.execute(
            "UPDATE visits SET scroll_y = ?1 WHERE id = ?2",
            params![y, visit.to_string()],
        )?;
        Ok(())
    }

    pub fn scroll(&self, visit: VisitId) -> Result<Option<i64>> {
        Ok(self.connection.query_row(
            "SELECT scroll_y FROM visits WHERE id = ?1",
            [visit.to_string()],
            |row| row.get(0),
        )?)
    }

    pub fn reload(&mut self, navigation: &mut Navigation) -> Result<Option<Url>> {
        navigation
            .current
            .as_ref()
            .map(|d| d.visit)
            .or(self.current_id()?)
            .map(|id| self.restore(navigation, id))
            .transpose()
    }

    pub fn commit_load(
        &mut self,
        navigation: &mut Navigation,
        ticket: LoadTicket,
        url: Url,
        visibility: Visibility,
    ) -> Result<bool> {
        let Pending::Loading {
            ticket: active,
            intent,
            ..
        } = &navigation.pending
        else {
            return Ok(false);
        };
        if *active != ticket {
            return Ok(false);
        }
        web_url(url.as_str())?;
        let intent = intent.clone();
        let previous = self.current_id()?;
        let kind = match &intent {
            Intent::New { .. } => "new",
            Intent::Restore { visit } if previous == Some(*visit) => "reload",
            Intent::Restore { .. } => "return",
        };
        let transaction = self.connection.transaction()?;
        let now = now_ms()?;
        let id = match intent {
            Intent::New { parent, source } => {
                let id = VisitId(Ulid::new());
                transaction.execute(
                    "INSERT INTO visits(id, parent_id, url, title, created_at, opened_at) VALUES (?1, ?2, ?3, '', ?4, ?4)",
                    params![id.to_string(), parent.map(|id| id.to_string()), url.as_str(), now],
                )?;
                if let Some(source) = source {
                    write_source(&transaction, id, &source)?;
                }
                id
            }
            Intent::Restore { visit } => {
                let changed = transaction.execute(
                    "UPDATE visits SET url = ?1, opened_at=COALESCE(opened_at, ?3) WHERE id = ?2",
                    params![url.as_str(), visit.to_string(), now],
                )?;
                if changed != 1 {
                    bail!("This visit no longer exists.")
                }
                visit
            }
        };
        if visibility == Visibility::Selected {
            record_selection(&transaction, id, previous, kind, now)?;
        }
        transaction.commit()?;
        navigation.publish(id, url);
        Ok(true)
    }

    pub fn select_document(&mut self, navigation: &Navigation, document: &Document) -> Result<()> {
        if !navigation.valid(document) {
            return Ok(());
        }
        let previous = self.current_id()?;
        if previous == Some(document.visit) {
            return Ok(());
        }
        let transaction = self.connection.transaction()?;
        record_selection(&transaction, document.visit, previous, "return", now_ms()?)?;
        transaction.commit()?;
        Ok(())
    }

    pub fn title_for_document(
        &mut self,
        navigation: &Navigation,
        document: &Document,
        title: &str,
    ) -> Result<()> {
        if !navigation.valid(document)
            || document.page_url != document.stored_url
            || title.trim().is_empty()
        {
            return Ok(());
        }
        self.connection.execute(
            "UPDATE visits SET title=?1 WHERE id=?2",
            params![title, document.visit.to_string()],
        )?;
        Ok(())
    }

    pub fn source_for_document(
        &mut self,
        navigation: &Navigation,
        document: &Document,
        child: VisitId,
        source: SourceContext,
    ) -> Result<bool> {
        if !navigation.valid(document) {
            return Ok(false);
        }
        self.set_source_context(child, document.visit, source)
    }

    pub fn capture_for_document(
        &mut self,
        navigation: &Navigation,
        document: &Document,
        payload: CapturePayload,
    ) -> Result<Option<i64>> {
        if !navigation.valid(document) {
            return Ok(None);
        }
        self.save_capture(document.visit, &document.stored_url, payload)
    }

    pub fn scroll_for_document(
        &mut self,
        navigation: &Navigation,
        document: &Document,
        y: i64,
    ) -> Result<()> {
        if navigation.valid(document) {
            self.set_scroll(document.visit, y)?;
        }
        Ok(())
    }

    pub fn queue_link(
        &mut self,
        parent: VisitId,
        url: Url,
        source: SourceContext,
    ) -> Result<VisitId> {
        web_url(url.as_str())?;
        let source = self.checked_source(parent, source)?;
        let id = VisitId(Ulid::new());
        let title = source.link_text.trim();
        let transaction = self.connection.transaction()?;
        transaction.execute(
            "INSERT INTO visits(id,parent_id,url,title,created_at,opened_at) VALUES(?1,?2,?3,?4,?5,NULL)",
            params![id.to_string(), parent.to_string(), url.as_str(), title, now_ms()?],
        )?;
        write_source(&transaction, id, &source)?;
        transaction.commit()?;
        Ok(id)
    }

    fn set_source_context(
        &mut self,
        child: VisitId,
        parent: VisitId,
        source: SourceContext,
    ) -> Result<bool> {
        let visit = self.visit(child)?;
        if visit.parent != Some(parent)
            || visit
                .source
                .as_ref()
                .is_none_or(|original| original.page_url != source.page_url)
        {
            return Ok(false);
        }
        let source = self.checked_source(parent, source)?;
        let transaction = self.connection.transaction()?;
        write_source(&transaction, child, &source)?;
        if visit.state == VisitState::Queued && !source.link_text.trim().is_empty() {
            transaction.execute(
                "UPDATE visits SET title=?1 WHERE id=?2",
                params![source.link_text, child.to_string()],
            )?;
        }
        transaction.commit()?;
        Ok(true)
    }

    pub fn follow_link(
        &mut self,
        navigation: &mut Navigation,
        parent: VisitId,
        url: Url,
        source: SourceContext,
    ) -> Result<Url> {
        web_url(url.as_str())?;
        let source = self.checked_source(parent, source)?;
        navigation.request(
            Intent::New {
                parent: Some(parent),
                source: Some(Box::new(source)),
            },
            url.clone(),
        );
        Ok(url)
    }

    pub fn open_citation(
        &mut self,
        navigation: &mut Navigation,
        child: VisitId,
    ) -> Result<Option<Url>> {
        let visit = self.visit(child)?;
        let Some(parent) = visit.parent else {
            return Ok(None);
        };
        let Some(source) = visit.source else {
            return Ok(None);
        };
        let Some(citation) = source.citation_url.clone() else {
            return Ok(None);
        };
        self.follow_link(navigation, parent, citation, source)
            .map(Some)
    }

    fn checked_source(&self, parent: VisitId, mut source: SourceContext) -> Result<SourceContext> {
        let visit = self.visit(parent)?;
        if visit.state != VisitState::Opened {
            bail!("The source of this link has not been opened yet.");
        }
        web_url(source.page_url.as_str())?;
        if visit.url.origin() != source.page_url.origin() {
            bail!("The source does not match the page on screen.");
        }
        check_citation(&source.page_url, source.citation_url.as_ref())?;
        source.link_text = bounded(&source.link_text, 300);
        source.excerpt = bounded(&source.excerpt, 1000);
        Ok(source)
    }

    fn save_capture(
        &mut self,
        visit: VisitId,
        expected_visit_url: &Url,
        mut payload: CapturePayload,
    ) -> Result<Option<i64>> {
        let visit = self.visit(visit)?;
        if visit.state != VisitState::Opened || &visit.url != expected_visit_url {
            return Ok(None);
        }
        web_url(payload.document_url.as_str())?;
        if payload.document_url.origin() != visit.url.origin() {
            return Ok(None);
        }
        check_citation(&payload.document_url, payload.citation_url.as_ref())?;
        payload.text = bounded(&payload.text, 200_000);
        if payload.text.is_empty() {
            return Ok(None);
        }
        payload.title = bounded(&payload.title, 500);
        if payload.title.is_empty() {
            payload.title = visit.label().to_owned();
        }
        if payload.kind == CaptureKind::Page {
            let latest: Option<i64> = self.connection.query_row(
                "SELECT id FROM captures WHERE visit_id=?1 AND kind='page' ORDER BY id DESC LIMIT 1",
                [visit.id.to_string()],
                |row| row.get(0),
            ).optional()?;
            if let Some(id) = latest {
                let latest = self.capture(id)?;
                if latest.document_url == payload.document_url
                    && latest.title == payload.title
                    && latest.text == payload.text
                    && latest.citation_url == payload.citation_url
                    && latest.source == visit.source
                {
                    return Ok(Some(id));
                }
            }
        }
        let source = visit.source.as_ref();
        self.connection.execute(
            "INSERT INTO captures(visit_id,document_url,citation_url,title,body,captured_at,kind,
                source_page_url,source_citation_url,source_link_text,source_excerpt)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
            params![
                visit.id.to_string(),
                payload.document_url.as_str(),
                payload.citation_url.as_ref().map(Url::as_str),
                payload.title,
                payload.text,
                now_ms()?,
                payload.kind.as_str(),
                source.map(|s| s.page_url.as_str()),
                source.and_then(|s| s.citation_url.as_ref().map(Url::as_str)),
                source.map(|s| s.link_text.as_str()),
                source.map(|s| s.excerpt.as_str())
            ],
        )?;
        Ok(Some(self.connection.last_insert_rowid()))
    }

    pub fn capture(&self, id: i64) -> Result<Capture> {
        let mut statement = self.connection.prepare(
            "SELECT id,visit_id,document_url,citation_url,title,body,captured_at,kind,
                source_page_url,source_citation_url,source_link_text,source_excerpt FROM captures WHERE id=?1",
        )?;
        let mut rows = statement.query([id])?;
        capture_from_row(rows.next()?.context("This saved text no longer exists.")?)
    }

    pub fn captures(&self, visit: VisitId) -> Result<Vec<Capture>> {
        let mut statement = self.connection.prepare(
            "SELECT id,visit_id,document_url,citation_url,title,body,captured_at,kind,
                source_page_url,source_citation_url,source_link_text,source_excerpt
             FROM captures WHERE visit_id=?1 ORDER BY id DESC",
        )?;
        let mut rows = statement.query([visit.to_string()])?;
        let mut captures = Vec::new();
        while let Some(row) = rows.next()? {
            captures.push(capture_from_row(row)?);
        }
        Ok(captures)
    }

    /// The page a download starting now belongs to. A navigation that turns into a download
    /// leaves no page of its own, so its file goes to the page that navigation came from: the
    /// parent it was given, or the parent of the pending page it tried to open. Any other
    /// download, and a navigation that came from no page, belongs to the page on screen.
    pub fn download_page(
        &self,
        navigation: &Navigation,
        request: Option<&Url>,
    ) -> Result<Option<VisitId>> {
        if let Pending::Loading { intent, url, .. } = &navigation.pending {
            if request == Some(url) {
                let from = match intent {
                    Intent::New { parent, .. } => *parent,
                    Intent::Restore { visit } => self
                        .connection
                        .query_row(
                            "SELECT parent_id FROM visits WHERE id=?1",
                            [visit.to_string()],
                            |row| row.get::<_, Option<String>>(0),
                        )
                        .optional()?
                        .flatten()
                        .map(|id| id.parse())
                        .transpose()?,
                };
                if from.is_some() {
                    return Ok(from);
                }
            }
        }
        self.current_id()
    }

    /// Attaches a saved file to the page it was downloaded from.
    pub fn record_download(
        &mut self,
        visit: VisitId,
        path: &Path,
        size: u64,
        source: Option<&Url>,
    ) -> Result<i64> {
        let path = path
            .to_str()
            .context("The saved file's path is not valid UTF-8.")?;
        let known: bool = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM visits WHERE id=?1)",
            [visit.to_string()],
            |row| row.get(0),
        )?;
        if !known {
            bail!("This visit no longer exists.");
        }
        self.connection.execute(
            "INSERT INTO downloads(visit_id, path, size, source_url, saved_at)
             VALUES(?1, ?2, ?3, ?4, ?5)",
            params![
                visit.to_string(),
                path,
                i64::try_from(size)?,
                source.map(Url::as_str),
                now_ms()?
            ],
        )?;
        Ok(self.connection.last_insert_rowid())
    }

    pub fn download(&self, id: i64) -> Result<Download> {
        let mut statement = self.connection.prepare(
            "SELECT id, visit_id, path, size, source_url, saved_at FROM downloads WHERE id=?1",
        )?;
        let mut rows = statement.query([id])?;
        download_from_row(rows.next()?.context("This download is no longer known.")?)
    }

    /// Every recorded download, oldest first.
    pub fn downloads(&self) -> Result<Vec<Download>> {
        let mut statement = self.connection.prepare(
            "SELECT id, visit_id, path, size, source_url, saved_at FROM downloads ORDER BY id",
        )?;
        let mut rows = statement.query([])?;
        let mut downloads = Vec::new();
        while let Some(row) = rows.next()? {
            downloads.push(download_from_row(row)?);
        }
        Ok(downloads)
    }

    pub fn set_theme_mode(&mut self, mode: ThemeMode) -> Result<()> {
        let value = match mode {
            ThemeMode::System => "system",
            ThemeMode::Light => "light",
            ThemeMode::Dark => "dark",
        };
        self.connection.execute(
            "UPDATE session SET theme_mode=?1 WHERE singleton=1",
            [value],
        )?;
        Ok(())
    }

    pub fn theme_mode(&self) -> Result<ThemeMode> {
        let mode: String = self.connection.query_row(
            "SELECT theme_mode FROM session WHERE singleton=1",
            [],
            |row| row.get(0),
        )?;
        Ok(match mode.as_str() {
            "light" => ThemeMode::Light,
            "dark" => ThemeMode::Dark,
            _ => ThemeMode::System,
        })
    }

    pub fn capture_enabled(&self) -> Result<bool> {
        Ok(self.connection.query_row(
            "SELECT capture_enabled FROM session WHERE singleton=1",
            [],
            |row| row.get(0),
        )?)
    }

    pub fn set_capture_enabled(&mut self, enabled: bool) -> Result<()> {
        self.connection.execute(
            "UPDATE session SET capture_enabled=?1 WHERE singleton=1",
            [enabled],
        )?;
        Ok(())
    }
}

fn now_ms() -> Result<i64> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis() as i64)
}

fn bounded(value: &str, limit: usize) -> String {
    value.trim().chars().take(limit).collect()
}

fn check_citation(document: &Url, citation: Option<&Url>) -> Result<()> {
    if let Some(citation) = citation {
        web_url(citation.as_str())?;
        if document.origin() != citation.origin() {
            bail!("The citation link must belong to the source site.");
        }
    }
    Ok(())
}

fn source_from_row(row: &Row<'_>, start: usize) -> Result<Option<SourceContext>> {
    let Some(page_url) = row.get::<_, Option<String>>(start)? else {
        return Ok(None);
    };
    Ok(Some(SourceContext {
        page_url: web_url(&page_url)?,
        citation_url: row
            .get::<_, Option<String>>(start + 1)?
            .map(|s| web_url(&s))
            .transpose()?,
        link_text: row.get::<_, Option<String>>(start + 2)?.unwrap_or_default(),
        excerpt: row.get::<_, Option<String>>(start + 3)?.unwrap_or_default(),
    }))
}

fn write_source(connection: &Connection, visit: VisitId, source: &SourceContext) -> Result<()> {
    connection.execute(
        "INSERT INTO origins(visit_id,source_page_url,source_citation_url,source_link_text,source_excerpt)
         VALUES(?1,?2,?3,?4,?5) ON CONFLICT(visit_id) DO UPDATE SET source_page_url=excluded.source_page_url,
            source_citation_url=excluded.source_citation_url,source_link_text=excluded.source_link_text,
            source_excerpt=excluded.source_excerpt",
        params![visit.to_string(), source.page_url.as_str(), source.citation_url.as_ref().map(Url::as_str), source.link_text, source.excerpt],
    )?;
    Ok(())
}

fn capture_from_row(row: &Row<'_>) -> Result<Capture> {
    let kind: String = row.get(7)?;
    let kind = match kind.as_str() {
        "page" => CaptureKind::Page,
        "selection" => CaptureKind::Selection,
        _ => bail!("Unknown saved text kind."),
    };
    Ok(Capture {
        id: row.get(0)?,
        visit: row.get::<_, String>(1)?.parse()?,
        document_url: web_url(&row.get::<_, String>(2)?)?,
        citation_url: row
            .get::<_, Option<String>>(3)?
            .map(|s| web_url(&s))
            .transpose()?,
        title: row.get(4)?,
        text: row.get(5)?,
        captured_at: row.get(6)?,
        kind,
        source: source_from_row(row, 8)?,
    })
}

fn download_from_row(row: &Row<'_>) -> Result<Download> {
    Ok(Download {
        id: row.get(0)?,
        visit: row.get::<_, String>(1)?.parse()?,
        path: row.get::<_, String>(2)?.into(),
        size: row.get::<_, i64>(3)?.max(0) as u64,
        source: row
            .get::<_, Option<String>>(4)?
            .map(|s| web_url(&s))
            .transpose()?,
        saved_at: row.get(5)?,
    })
}

/// An imported page from a search row, or `None` when its URL is not a web address.
fn imported_row(row: &Row) -> rusqlite::Result<Option<ImportedPage>> {
    let url: String = row.get(0)?;
    let Ok(url) = web_url(&url) else {
        return Ok(None);
    };
    Ok(Some(ImportedPage {
        url,
        title: row.get(1)?,
        visits: row.get::<_, i64>(2)?.max(0) as usize,
        typed: row.get::<_, i64>(3)?.max(0) as usize,
        last_visit: row.get(4)?,
    }))
}

fn record_selection(
    transaction: &rusqlite::Transaction<'_>,
    id: VisitId,
    previous: Option<VisitId>,
    kind: &str,
    now: i64,
) -> Result<()> {
    transaction.execute(
        "INSERT INTO visit_events(visit_id, from_visit_id, kind, at) VALUES (?1, ?2, ?3, ?4)",
        params![id.to_string(), previous.map(|id| id.to_string()), kind, now],
    )?;
    transaction.execute(
        "UPDATE session SET current_id = ?1 WHERE singleton = 1",
        [id.to_string()],
    )?;
    transaction.execute(
        "WITH RECURSIVE ancestry(id, parent_id) AS (
                SELECT id, parent_id FROM visits WHERE id=?1
                UNION
                SELECT v.id, v.parent_id FROM visits v JOIN ancestry a ON v.id=a.parent_id
             )
             INSERT INTO tree_state(root_id, last_visit_id)
                SELECT id, ?1 FROM ancestry WHERE parent_id IS NULL
             ON CONFLICT(root_id) DO UPDATE SET last_visit_id=excluded.last_visit_id, settled_at=NULL",
        [id.to_string()],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestBrowser {
        browser: Browser,
        navigation: Navigation,
    }
    impl std::ops::Deref for TestBrowser {
        type Target = Browser;
        fn deref(&self) -> &Browser {
            &self.browser
        }
    }
    impl std::ops::DerefMut for TestBrowser {
        fn deref_mut(&mut self) -> &mut Browser {
            &mut self.browser
        }
    }
    impl TestBrowser {
        fn open(path: &Path) -> Result<Self> {
            Ok(Self {
                browser: Browser::open(path)?,
                navigation: Navigation::default(),
            })
        }
        fn with_connection(connection: Connection) -> Result<Self> {
            Ok(Self {
                browser: Browser::with_connection(connection)?,
                navigation: Navigation::default(),
            })
        }
        fn navigate(&mut self, url: Url, target: NavigationTarget) -> Result<Url> {
            self.browser.navigate(&mut self.navigation, url, target)
        }
        fn restore(&mut self, id: VisitId) -> Result<Url> {
            self.browser.restore(&mut self.navigation, id)
        }
        fn back(&mut self) -> Result<Option<Url>> {
            self.browser.back(&mut self.navigation)
        }
        fn reload(&mut self) -> Result<Option<Url>> {
            self.browser.reload(&mut self.navigation)
        }
        fn follow_link(&mut self, parent: VisitId, url: Url, source: SourceContext) -> Result<Url> {
            self.browser
                .follow_link(&mut self.navigation, parent, url, source)
        }
        fn open_citation(&mut self, child: VisitId) -> Result<Option<Url>> {
            self.browser.open_citation(&mut self.navigation, child)
        }
        // These SQLite tests model an accepted request followed by Started. They have no
        // native signal queue; navigation_smoke covers policy and Started interleavings.
        fn started(&mut self, url: Url) -> Result<()> {
            let ticket = self
                .navigation
                .pending_ticket()
                .unwrap_or_else(|| self.navigation.native_request(url.clone()));
            self.navigation.started(ticket);
            Ok(())
        }
        fn committed(&mut self, url: Url) -> Result<bool> {
            let Some(ticket) = self.navigation.pending_ticket() else {
                return Ok(false);
            };
            self.browser
                .commit_load(&mut self.navigation, ticket, url, Visibility::Selected)
        }
        fn failed(&mut self, _url: &Url, cancelled: bool) {
            if !cancelled {
                if let Some(ticket) = self.navigation.pending_ticket() {
                    self.navigation.failed(ticket);
                }
            }
        }
        fn redirected(&mut self, url: Url) {
            if let Some(ticket) = self.navigation.pending_ticket() {
                self.navigation.redirected(ticket, url);
            }
        }
        fn title_changed(&mut self, url: &Url, title: &str) -> Result<()> {
            if let Some(document) = self
                .navigation
                .current
                .as_ref()
                .filter(|d| &d.page_url == url)
            {
                self.browser
                    .title_for_document(&self.navigation, document, title)?;
            }
            Ok(())
        }
        fn requested_restore(&self) -> Option<VisitId> {
            self.browser.requested_restore(&self.navigation)
        }
        // The view owner drops its load when the window is left with no page.
        fn settle_tree(&mut self, tree: TreeId) -> Result<Landing> {
            let landing = self.browser.settle_tree(tree)?;
            if landing == Landing::Start {
                self.navigation.clear();
            }
            Ok(landing)
        }
        fn download_page(&self, url: Option<&Url>) -> Result<Option<VisitId>> {
            self.browser.download_page(&self.navigation, url)
        }
    }

    fn url(path: &str) -> Url {
        Url::parse(&format!("https://example.org/{path}")).unwrap()
    }
    fn memory() -> TestBrowser {
        TestBrowser::with_connection(Connection::open_in_memory().unwrap()).unwrap()
    }
    fn follow(browser: &mut TestBrowser, path: &str, target: NavigationTarget) -> VisitId {
        let url = browser.navigate(url(path), target).unwrap();
        browser.started(url.clone()).unwrap();
        assert!(browser.committed(url).unwrap());
        browser.snapshot().unwrap().current.unwrap()
    }

    fn follow_child(browser: &mut TestBrowser, path: &str) -> VisitId {
        let parent = browser.current_id().unwrap().unwrap();
        follow(browser, path, NavigationTarget::ChildOf(parent))
    }

    fn historical_connection(version: i64) -> Connection {
        let connection = Connection::open_in_memory().unwrap();
        let sql = include_str!("../tests/fixtures/schema-v8.sql");
        connection.execute_batch(sql).unwrap();
        if version >= 9 {
            let tx = connection.unchecked_transaction().unwrap();
            migrate_pins(&tx).unwrap();
            tx.commit().unwrap();
        }
        if version >= 10 {
            let tx = connection.unchecked_transaction().unwrap();
            bookmarks::migrate(&tx).unwrap();
            tx.commit().unwrap();
        }
        connection
    }

    #[test]
    fn chrome_fill_survives_bookmarks_and_declines_ahead_of_it() {
        let mut b = memory();
        let pages: Vec<_> = (0..12)
            .map(|i| ImportedPage {
                url: web_url(&format!("https://site{i}.test/")).unwrap(),
                title: format!("Site {i}"),
                visits: 100 - i,
                typed: 0,
                last_visit: None,
            })
            .collect();
        b.replace_imported("chrome", &pages).unwrap();
        for page in &pages[..8] {
            b.bookmark(&page.url, &page.title).unwrap();
        }
        b.decline(&crate::suggest::SuggestionKey::Page(pages[8].url.clone()))
            .unwrap();
        let bookmarks = b.bookmarks().unwrap();
        let declined = b.declined().unwrap();
        let chrome = b
            .chrome_favorites(3 + bookmarks.len() + declined.len())
            .unwrap();
        let forest = crate::forest::ForestProjection::from_snapshot(
            b.snapshot().unwrap(),
            b.tree_states().unwrap(),
        )
        .unwrap();
        let result = crate::suggest::rank(
            &crate::suggest::Inputs {
                forest: &forest,
                bookmarks: &bookmarks,
                declined: &declined,
                arrivals: &[],
                chrome: &chrome,
                left: None,
            },
            100,
        );
        assert_eq!(result.len(), 3);
        assert!(result.iter().all(|s| s.why() == "Often in Chrome"));
    }

    #[test]
    fn opens_a_real_version_10_database_and_declines_persist() {
        let connection = historical_connection(10);
        assert!(connection
            .prepare("SELECT key FROM declined_suggestions")
            .is_err());
        let mut browser = Browser::with_connection(connection).unwrap();
        let key = crate::suggest::SuggestionKey::Page(url("declined"));
        browser.decline(&key).unwrap();
        browser.decline(&key).unwrap();
        let mut browser = Browser::with_connection(browser.connection).unwrap();
        assert_eq!(browser.declined().unwrap(), HashSet::from([key]));
        browser.bookmark(&url("kept"), "First title").unwrap();
        browser.bookmark(&url("kept"), "Second title").unwrap();
        assert_eq!(browser.bookmarks().unwrap().len(), 1);
        assert_eq!(browser.bookmarks().unwrap()[0].title, "First title");
    }

    #[test]
    fn seeded_history_pins_ranked_suggestions() {
        let b = memory();
        let now = 100 * crate::suggest::DAY;
        for i in 1..=40_u128 {
            let id = ulid::Ulid::from(i).to_string();
            b.connection
                .execute(
                    "INSERT INTO visits(id,url,title,created_at,opened_at) VALUES(?1,?2,?3,?4,?4)",
                    params![
                        id,
                        format!("https://seed{i}.test/"),
                        format!("Seed {i}"),
                        now - 10 * crate::suggest::DAY
                    ],
                )
                .unwrap();
            b.connection
                .execute(
                    "INSERT INTO tree_state(root_id,last_visit_id,settled_at) VALUES(?1,?1,?2)",
                    params![id, now],
                )
                .unwrap();
            for day in 1..=3 {
                b.connection
                    .execute(
                        "INSERT INTO visit_events(visit_id,kind,at) VALUES(?1,'return',?2)",
                        params![id, now - day * crate::suggest::DAY],
                    )
                    .unwrap();
            }
        }
        let forest = crate::forest::ForestProjection::from_snapshot(
            b.snapshot().unwrap(),
            b.tree_states().unwrap(),
        )
        .unwrap();
        let arrivals = b.arrivals(&forest, 0).unwrap();
        let result = crate::suggest::rank(
            &crate::suggest::Inputs {
                forest: &forest,
                bookmarks: &[],
                declined: &HashSet::new(),
                arrivals: &arrivals,
                chrome: &[],
                left: b.last_event_tree(&forest).unwrap(),
            },
            now,
        );
        assert_eq!(
            result
                .iter()
                .map(|s| (s.key().to_string(), s.why()))
                .collect::<Vec<_>>(),
            vec![
                (
                    "tree 00000000000000000000000001".into(),
                    "Opened 3 days in a row".into()
                ),
                (
                    "tree 00000000000000000000000002".into(),
                    "Opened 3 days in a row".into()
                ),
                (
                    "tree 00000000000000000000000003".into(),
                    "Opened 3 days in a row".into()
                ),
            ]
        );
    }

    #[test]
    fn history_counts_returns_but_never_reloads_and_finds_the_last_tree_outside_the_window() {
        let mut b = memory();
        let root = follow(&mut b, "root", NavigationTarget::Root);
        let forest = crate::forest::ForestProjection::from_snapshot(
            b.snapshot().unwrap(),
            b.tree_states().unwrap(),
        )
        .unwrap();
        let before = b.arrivals(&forest, 0).unwrap().len();
        let load = b.reload().unwrap().unwrap();
        b.started(load.clone()).unwrap();
        b.committed(load).unwrap();
        assert_eq!(b.arrivals(&forest, 0).unwrap().len(), before);
        let child = follow_child(&mut b, "child");
        let load = b.restore(root).unwrap();
        b.started(load.clone()).unwrap();
        b.committed(load).unwrap();
        let forest = crate::forest::ForestProjection::from_snapshot(
            b.snapshot().unwrap(),
            b.tree_states().unwrap(),
        )
        .unwrap();
        let arrivals = b.arrivals(&forest, 0).unwrap();
        assert_eq!(arrivals.len(), before + 2);
        assert_eq!(arrivals.last().unwrap().kind, ArrivalKind::Return);
        assert_eq!(arrivals.last().unwrap().from, Some(child));
        b.connection
            .execute("UPDATE visit_events SET at=1", [])
            .unwrap();
        assert!(b.arrivals(&forest, 2).unwrap().is_empty());
        assert_eq!(b.last_event_tree(&forest).unwrap(), Some(TreeId(root)));
    }

    #[test]
    fn the_tree_just_left_follows_the_order_of_the_log_not_the_clock() {
        let mut b = memory();
        let old = follow(&mut b, "old", NavigationTarget::Root);
        let recent = follow(&mut b, "recent", NavigationTarget::Root);
        b.connection
            .execute(
                "UPDATE visit_events SET at=at+7200000 WHERE visit_id=?1",
                [old.to_string()],
            )
            .unwrap();
        let forest = crate::forest::ForestProjection::from_snapshot(
            b.snapshot().unwrap(),
            b.tree_states().unwrap(),
        )
        .unwrap();
        assert_eq!(b.last_event_tree(&forest).unwrap(), Some(TreeId(recent)));
    }

    #[test]
    fn opens_a_version_9_database_and_copies_chrome_bookmarks_once() {
        let connection = historical_connection(9);
        for (i, folder) in [
            "Bookmarks bar",
            "Bookmarks bar",
            "Bookmarks bar › Work",
            "Bookmarks bar › Work › Rust",
            "Other bookmarks",
            "Mobile bookmarks",
        ]
        .iter()
        .enumerate()
        {
            connection.execute("INSERT INTO imported_pages(source,url,title,visits,typed,last_visit,folder) VALUES('chrome',?1,?2,?3,0,NULL,?4)", params![format!("https://example.org/{i}"), format!("Title {i}"), i as i64, folder]).unwrap();
        }
        connection
            .execute(
                "INSERT INTO imported_pages(source,url,title,visits,typed,last_visit,folder)
                 VALUES('chrome','file:///home/me/notes.html','Notes',0,0,NULL,'Bookmarks bar')",
                [],
            )
            .unwrap();
        let mut browser = Browser::with_connection(connection).unwrap();
        let bookmarks = browser.bookmarks().unwrap();
        assert_eq!(bookmarks.len(), 6);
        assert_eq!(
            browser
                .connection
                .query_row(
                    "SELECT COUNT(*) FROM imported_pages WHERE url LIKE 'file:%'",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            1,
            "a bookmark that is not a web address keeps its imported row"
        );
        assert_eq!(bookmarks[0].url.path(), "/1");
        assert_eq!(bookmarks[1].url.path(), "/0");
        assert_eq!(
            bookmarks
                .iter()
                .map(|b| b.folder.as_deref())
                .collect::<Vec<_>>(),
            [
                None,
                None,
                Some("Mobile bookmarks"),
                Some("Other bookmarks"),
                Some("Work"),
                Some("Work › Rust")
            ]
        );
        assert_eq!(browser.search_imported("example", 20).unwrap().len(), 5);
        assert!(browser
            .connection
            .prepare("SELECT folder FROM imported_pages")
            .is_err());
        browser.remove_bookmark(bookmarks[0].id).unwrap();
        let reopened = Browser::with_connection(browser.connection).unwrap();
        assert_eq!(reopened.bookmarks().unwrap().len(), 5);
        assert!(!reopened
            .bookmarks()
            .unwrap()
            .iter()
            .any(|b| b.url == bookmarks[0].url));
    }

    #[test]
    fn toggle_current_bookmark_adds_removes_and_needs_a_page() {
        let mut b = memory();
        assert_eq!(b.toggle_current_bookmark().unwrap(), BookmarkToggle::NoPage);
        let id = follow(&mut b, "chosen", NavigationTarget::Root);
        assert_eq!(b.toggle_current_bookmark().unwrap(), BookmarkToggle::Added);
        let bookmark = b.bookmarks().unwrap().remove(0);
        let visit = b
            .snapshot()
            .unwrap()
            .visits
            .into_iter()
            .find(|v| v.id == id)
            .unwrap();
        assert_eq!((bookmark.url, bookmark.title), (visit.url, visit.title));
        assert_eq!(
            b.toggle_current_bookmark().unwrap(),
            BookmarkToggle::Removed
        );
        assert!(b.bookmarks().unwrap().is_empty());
        b.toggle_current_bookmark().unwrap();
        assert!(b.bookmarks().unwrap()[0].id.get() > bookmark.id.get());
    }

    #[test]
    fn a_failed_bookmark_import_rolls_back_history_replacement() {
        let mut b = memory();
        let history = ImportedPage {
            url: url("history"),
            title: "History".into(),
            visits: 1,
            typed: 0,
            last_visit: None,
        };
        b.import_pages("chrome", std::slice::from_ref(&history), &[])
            .unwrap();
        b.connection.execute_batch("CREATE TRIGGER reject_bookmark BEFORE INSERT ON bookmarks BEGIN SELECT RAISE(ABORT, 'fixture failure'); END;").unwrap();
        let bookmark = ImportedBookmark {
            url: url("bookmark"),
            title: String::new(),
            folder: "Bookmarks bar".into(),
            visits: 0,
        };
        assert!(b.import_pages("chrome", &[], &[bookmark]).is_err());
        assert_eq!(b.search_imported("history", 10).unwrap(), [history]);
        assert!(b.bookmarks().unwrap().is_empty());
    }

    #[test]
    fn import_bookmarks_appends_absent_ones_at_the_end_of_their_folder() {
        let mut b = memory();
        let draft = |path: &str, folder: &str, visits| ImportedBookmark {
            url: url(path),
            title: path.into(),
            folder: folder.into(),
            visits,
        };
        b.import_pages(
            "chrome",
            &[],
            &[
                draft("first", "Bookmarks bar", 5),
                draft("second", "Bookmarks bar", 1),
                draft("nested", "Bookmarks bar › Work › Rust", 1),
            ],
        )
        .unwrap();
        let original = b.bookmarks().unwrap();
        b.remove_bookmark(original[1].id).unwrap();
        b.import_pages(
            "chrome",
            &[],
            &[
                draft("first", "Other bookmarks", 100),
                draft("third", "Bookmarks bar", 100),
            ],
        )
        .unwrap();
        let rows = b.bookmarks().unwrap();
        assert_eq!(rows[0], original[0]);
        assert_eq!(rows[1].url, url("third"));
        assert_eq!(rows[2], original[2]);
        assert_eq!(
            b.import_pages("chrome", &[], &[draft("third", "Bookmarks bar", 100)])
                .unwrap(),
            0
        );
        let tx = b.connection.transaction().unwrap();
        assert!(tx.execute("INSERT INTO bookmarks(url,title,folder,position,added_at) VALUES('https://invalid/','',' ',0,0)", []).is_err());
    }

    #[test]
    fn opens_a_version_8_database_with_every_tree_unpinned() {
        let connection = historical_connection(8);
        let root = Ulid::new().to_string();
        connection.execute("INSERT INTO visits(id,url,created_at,opened_at) VALUES(?1,'https://example.org/',1,1)", [&root]).unwrap();
        connection
            .execute(
                "INSERT INTO tree_state(root_id,last_visit_id,settled_at) VALUES(?1,?1,2)",
                [&root],
            )
            .unwrap();
        let b = Browser::with_connection(connection).unwrap();
        let states = b.tree_states().unwrap();
        assert_eq!(states.len(), 1);
        assert!(states[0].pin.is_none());
        assert_eq!(states[0].last_event_at, None);
        assert!(!states[0].lifecycle.is_open());
        assert_eq!(
            b.connection
                .pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
                .unwrap(),
            11
        );
    }

    #[test]
    fn pin_order_is_stable_idempotent_and_independent_of_lifecycle() {
        let mut b = memory();
        let a = follow(&mut b, "a", NavigationTarget::Root);
        let child = follow_child(&mut b, "child");
        let c = follow(&mut b, "c", NavigationTarget::Root);
        b.set_pinned(TreeId(c), true).unwrap();
        b.set_pinned(TreeId(a), true).unwrap();
        let pins = b.tree_states().unwrap();
        b.set_pinned(TreeId(c), true).unwrap();
        assert_eq!(b.tree_states().unwrap(), pins);
        b.settle_tree(TreeId(a)).unwrap();
        assert_eq!(b.tree_states().unwrap()[0].pin, pins[0].pin);
        b.restore(child).unwrap();
        let request = url("child");
        b.started(request.clone()).unwrap();
        b.committed(request).unwrap();
        let states = b.tree_states().unwrap();
        assert!(states[0].lifecycle.is_open());
        assert_eq!(states[0].pin, pins[0].pin);
        let projection =
            crate::forest::ForestProjection::from_snapshot(b.snapshot().unwrap(), states).unwrap();
        assert_eq!(
            projection.pinned().iter().map(|t| t.id).collect::<Vec<_>>(),
            vec![TreeId(c), TreeId(a)]
        );
        b.set_pinned(TreeId(c), false).unwrap();
        b.set_pinned(TreeId(c), true).unwrap();
        assert!(b.tree_states().unwrap()[1].pin > b.tree_states().unwrap()[0].pin);
    }

    #[test]
    fn tree_age_uses_the_newest_event_anywhere_in_the_tree_including_reload() {
        let mut b = memory();
        let a = follow(&mut b, "a", NavigationTarget::Root);
        let child = follow_child(&mut b, "child");
        b.connection
            .execute("UPDATE visit_events SET at=10", [])
            .unwrap();
        b.connection
            .execute(
                "INSERT INTO visit_events(visit_id,kind,at) VALUES(?1,'reload',100)",
                [a.to_string()],
            )
            .unwrap();
        let state = &b.tree_states().unwrap()[0];
        assert_eq!(state.last_visit, child);
        assert_eq!(state.last_event_at, Some(100));
    }

    #[test]
    fn interleaved_views_ticket_cancellation_redirect_and_hidden_commit() {
        let mut browser = Browser::with_connection(Connection::open_in_memory().unwrap()).unwrap();
        let mut a = Navigation::default();
        let mut b = Navigation::default();
        browser
            .navigate(&mut a, url("same"), NavigationTarget::Root)
            .unwrap();
        browser
            .navigate(&mut b, url("same"), NavigationTarget::Root)
            .unwrap();
        let first = a.pending_ticket().unwrap();
        let other = b.pending_ticket().unwrap();
        a.started(first);
        b.started(other);
        a.failed(first);
        assert_eq!(b.pending_ticket(), Some(other));
        assert!(!browser
            .commit_load(&mut a, first, url("same"), Visibility::Selected)
            .unwrap());
        browser
            .navigate(&mut a, url("same"), NavigationTarget::Root)
            .unwrap();
        let cancelled = first;
        let first = a.pending_ticket().unwrap();
        a.started(first);
        a.failed(cancelled);
        a.failed(other);
        b.redirected(first, url("wrong"));
        a.redirected(first, url("final"));
        assert!(browser
            .commit_load(&mut a, first, url("final"), Visibility::Selected)
            .unwrap());
        let before = browser.snapshot().unwrap();
        let targets = browser.tree_states().unwrap();
        let arrivals = events(&browser);
        assert!(browser
            .commit_load(&mut b, other, url("same"), Visibility::Hidden)
            .unwrap());
        assert_eq!(browser.snapshot().unwrap().current, before.current);
        assert_eq!(browser.tree_states().unwrap(), targets);
        assert_eq!(events(&browser), arrivals);
        let document = b.current.clone().unwrap();
        browser
            .title_for_document(&b, &document, "hidden title")
            .unwrap();
        assert_eq!(browser.visit(document.visit).unwrap().title, "hidden title");
        assert!(!browser
            .commit_load(&mut b, other, url("same"), Visibility::Selected)
            .unwrap());
        browser
            .navigate(&mut a, url("same"), NavigationTarget::Root)
            .unwrap();
        let replacement = a.pending_ticket().unwrap();
        a.started(first);
        a.failed(first);
        assert!(!browser
            .commit_load(&mut a, first, url("same"), Visibility::Selected)
            .unwrap());
        assert_eq!(a.pending_ticket(), Some(replacement));
        a.failed(replacement);
        browser.select_document(&b, &document).unwrap();
        assert_eq!(browser.snapshot().unwrap().current, Some(document.visit));
    }

    #[test]
    fn replaced_document_capture_and_scroll_are_dropped() {
        let mut browser = memory();
        follow(&mut browser, "same", NavigationTarget::Root);
        let old = browser.navigation.current.clone().unwrap();
        let source = SourceContext {
            page_url: old.page_url.clone(),
            citation_url: None,
            link_text: "initial".into(),
            excerpt: "initial excerpt".into(),
        };
        let queued = browser
            .queue_link(old.visit, url("queued"), source.clone())
            .unwrap();
        let replacement = follow(&mut browser, "same", NavigationTarget::Root);
        let mut late_source = source.clone();
        late_source.excerpt = "late enrichment".into();
        assert!(!browser
            .browser
            .source_for_document(&browser.navigation, &old, queued, late_source)
            .unwrap());
        assert_eq!(browser.visit(queued).unwrap().source, Some(source));
        assert_eq!(
            browser
                .browser
                .capture_for_document(
                    &browser.navigation,
                    &old,
                    payload("same", "late capture", CaptureKind::Page)
                )
                .unwrap(),
            None
        );
        browser
            .browser
            .scroll_for_document(&browser.navigation, &old, 900)
            .unwrap();
        assert_eq!(browser.scroll(replacement).unwrap(), None);
        assert!(browser.search("late capture").unwrap().is_empty());
    }

    #[test]
    fn failed_commit_preserves_document_and_ticket_for_retry() {
        let mut browser = memory();
        let root = follow(&mut browser, "root", NavigationTarget::Root);
        let old = browser.navigation.current.clone();
        let request = browser
            .navigate(url("child"), NavigationTarget::ChildOf(root))
            .unwrap();
        browser.started(request.clone()).unwrap();
        let ticket = browser.navigation.pending_ticket();
        let before = browser.snapshot().unwrap();
        let arrivals = events(&browser);
        browser.connection.execute_batch("CREATE TRIGGER reject_commit BEFORE UPDATE ON tree_state BEGIN SELECT RAISE(ABORT, 'rollback'); END;").unwrap();
        assert!(browser.committed(request.clone()).is_err());
        assert_eq!(browser.navigation.current, old);
        assert_eq!(browser.navigation.pending_ticket(), ticket);
        assert_eq!(browser.snapshot().unwrap(), before);
        assert_eq!(events(&browser), arrivals);
        browser
            .connection
            .execute_batch("DROP TRIGGER reject_commit")
            .unwrap();
        assert!(browser.committed(request).unwrap());
    }

    #[test]
    fn replaced_document_title_cannot_retitle_equal_url() {
        let mut browser = memory();
        let first = follow(&mut browser, "same", NavigationTarget::Root);
        let old_document = browser.navigation.current.clone().unwrap();
        assert_eq!(old_document.visit, first);
        let replacement = follow(&mut browser, "same", NavigationTarget::Root);
        browser
            .browser
            .title_for_document(&browser.navigation, &old_document, "late old title")
            .unwrap();
        assert_eq!(browser.visit(replacement).unwrap().title, "");
    }

    #[test]
    fn queueing_a_link_in_a_settled_tree_does_not_reopen_it() {
        let mut b = memory();
        let a = follow(&mut b, "a", NavigationTarget::Root);
        b.settle_tree(TreeId(a)).unwrap();
        let states = b.tree_states().unwrap();
        b.queue_link(a, url("queued"), source("context")).unwrap();
        assert_eq!(b.tree_states().unwrap(), states);
    }

    #[test]
    fn settled_trees_stay_settled_after_restart_and_v7_forests_open_with_every_tree_open() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("forest.db");
        let a = {
            let mut b = TestBrowser::open(&path).unwrap();
            let a = follow(&mut b, "a", NavigationTarget::Root);
            b.settle_tree(TreeId(a)).unwrap();
            a
        };
        let b = Browser::open(&path).unwrap();
        assert!(!b.tree_states().unwrap()[0].lifecycle.is_open());
        assert_eq!(b.visit(a).unwrap().url, url("a"));
        drop(b);
        let legacy = dir.path().join("legacy.db");
        let connection = Connection::open(&legacy).unwrap();
        connection
            .execute_batch(
                &include_str!("../tests/fixtures/schema-v8.sql")
                    .replace(", settled_at INTEGER CHECK(settled_at > 0)", "")
                    .replace("PRAGMA user_version=8", "PRAGMA user_version=7"),
            )
            .unwrap();
        connection.execute("INSERT INTO visits(id,url,created_at,opened_at) VALUES(?1,'https://example.org/a',1,1)", [a.to_string()]).unwrap();
        connection
            .execute(
                "INSERT INTO tree_state(root_id,last_visit_id) VALUES(?1,?1)",
                [a.to_string()],
            )
            .unwrap();
        drop(connection);
        assert!(Browser::open(&legacy).unwrap().tree_states().unwrap()[0]
            .lifecycle
            .is_open());
    }

    #[test]
    fn a_failed_reopen_commit_rolls_back_with_the_cursor() {
        let mut b = memory();
        let a = follow(&mut b, "a", NavigationTarget::Root);
        b.settle_tree(TreeId(a)).unwrap();
        let before = b.snapshot().unwrap();
        let states = b.tree_states().unwrap();
        b.connection.execute_batch("CREATE TRIGGER reject_target BEFORE UPDATE ON tree_state BEGIN SELECT RAISE(ABORT, 'test failure'); END;").unwrap();
        let request = b.restore(a).unwrap();
        b.started(request.clone()).unwrap();
        assert!(b.committed(request).is_err());
        assert_eq!(b.snapshot().unwrap(), before);
        assert_eq!(b.tree_states().unwrap(), states);
    }

    #[test]
    fn the_current_page_stays_in_a_settled_tree_until_the_landing_commits() {
        let mut b = memory();
        let a = follow(&mut b, "a", NavigationTarget::Root);
        let c = follow(&mut b, "c", NavigationTarget::Root);
        b.settle_tree(TreeId(c)).unwrap();
        let request = b.restore(a).unwrap();
        b.started(request.clone()).unwrap();
        assert_eq!(b.snapshot().unwrap().current, Some(c));
        assert!(!b.tree_states().unwrap()[1].lifecycle.is_open());
        b.committed(request).unwrap();
        assert_eq!(b.snapshot().unwrap().current, Some(a));
    }
    #[test]
    fn settling_another_tree_hides_it_and_keeps_every_page_and_the_current_page() {
        let mut b = memory();
        let a = follow(&mut b, "a", NavigationTarget::Root);
        let c = follow(&mut b, "c", NavigationTarget::Root);
        assert_eq!(b.settle_tree(TreeId(a)).unwrap(), Landing::Unchanged);
        assert_eq!(b.snapshot().unwrap().visits.len(), 2);
        assert_eq!(b.snapshot().unwrap().current, Some(c));
        assert!(!b.tree_states().unwrap()[0].lifecycle.is_open());
    }

    #[test]
    fn settling_the_current_tree_lands_on_the_open_tree_visited_last_not_created_last() {
        let mut b = memory();
        let a = follow(&mut b, "a", NavigationTarget::Root);
        follow(&mut b, "b", NavigationTarget::Root);
        let request = b.restore(a).unwrap();
        b.started(request.clone()).unwrap();
        b.committed(request).unwrap();
        let c = follow(&mut b, "c", NavigationTarget::Root);
        assert_eq!(b.settle_tree(TreeId(c)).unwrap(), Landing::Restore(a));
        assert_eq!(b.snapshot().unwrap().current, Some(c));
    }

    #[test]
    fn a_failed_landing_keeps_the_tree_settled_and_settling_again_retries_it() {
        let mut b = memory();
        let a = follow(&mut b, "a", NavigationTarget::Root);
        let c = follow(&mut b, "c", NavigationTarget::Root);
        assert_eq!(b.settle_tree(TreeId(c)).unwrap(), Landing::Restore(a));
        let states = b.tree_states().unwrap();
        let request = b.restore(a).unwrap();
        b.started(request.clone()).unwrap();
        b.failed(&request, false);
        assert_eq!(b.snapshot().unwrap().current, Some(c));
        assert_eq!(b.settle_tree(TreeId(c)).unwrap(), Landing::Restore(a));
        assert_eq!(b.tree_states().unwrap(), states);
    }

    #[test]
    fn settling_the_last_open_tree_clears_the_current_page_and_ignores_its_inflight_load() {
        let mut b = memory();
        let a = follow(&mut b, "a", NavigationTarget::Root);
        let request = b
            .navigate(url("child"), NavigationTarget::ChildOf(a))
            .unwrap();
        b.started(request.clone()).unwrap();
        assert_eq!(b.settle_tree(TreeId(a)).unwrap(), Landing::Start);
        assert!(!b.committed(request).unwrap());
        assert_eq!(b.snapshot().unwrap().current, None);
        assert_eq!(b.snapshot().unwrap().visits.len(), 1);
    }

    #[test]
    fn settling_the_last_open_tree_before_the_landing_commits_clears_the_current_page() {
        let mut b = memory();
        let a = follow(&mut b, "a", NavigationTarget::Root);
        let c = follow(&mut b, "c", NavigationTarget::Root);
        assert_eq!(b.settle_tree(TreeId(c)).unwrap(), Landing::Restore(a));
        assert_eq!(b.settle_tree(TreeId(a)).unwrap(), Landing::Start);
        assert_eq!(b.snapshot().unwrap().current, None);
        assert_eq!(b.snapshot().unwrap().visits.len(), 2);
    }

    #[test]
    fn a_load_in_flight_in_the_current_tree_cannot_bring_its_tab_back_once_the_landing_is_requested(
    ) {
        let mut b = memory();
        let a = follow(&mut b, "a", NavigationTarget::Root);
        let c = follow(&mut b, "c", NavigationTarget::Root);
        let request = b
            .navigate(url("child"), NavigationTarget::ChildOf(c))
            .unwrap();
        b.started(request.clone()).unwrap();
        assert_eq!(b.settle_tree(TreeId(c)).unwrap(), Landing::Restore(a));
        b.restore(a).unwrap();
        assert!(!b.committed(request).unwrap());
        assert!(!b.tree_states().unwrap()[1].lifecycle.is_open());
    }

    #[test]
    fn a_page_already_loading_in_another_tree_arrives_and_brings_its_tab_back() {
        let mut b = memory();
        let a = follow(&mut b, "a", NavigationTarget::Root);
        follow(&mut b, "c", NavigationTarget::Root);
        let request = b.restore(a).unwrap();
        b.started(request.clone()).unwrap();
        assert_eq!(b.settle_tree(TreeId(a)).unwrap(), Landing::Unchanged);
        assert!(b.committed(request).unwrap());
        assert!(b.tree_states().unwrap()[0].lifecycle.is_open());
    }

    #[test]
    fn restoring_reloading_or_following_a_link_in_a_settled_tree_reopens_it() {
        for action in 0..3 {
            let mut b = memory();
            let a = follow(&mut b, "a", NavigationTarget::Root);
            follow(&mut b, "c", NavigationTarget::Root);
            b.settle_tree(TreeId(a)).unwrap();
            let request = b.restore(a).unwrap();
            b.started(request.clone()).unwrap();
            if action != 0 {
                b.committed(request.clone()).unwrap();
                b.settle_tree(TreeId(a)).unwrap();
            }
            let request = match action {
                1 => b.reload().unwrap().unwrap(),
                2 => b
                    .navigate(url("child"), NavigationTarget::ChildOf(a))
                    .unwrap(),
                _ => request,
            };
            if action != 0 {
                b.started(request.clone()).unwrap();
            }
            assert!(b.committed(request).unwrap());
            assert!(b.tree_states().unwrap()[0].lifecycle.is_open());
        }
    }

    #[test]
    fn a_planted_tree_never_loaded_is_the_last_choice_for_landing() {
        let mut b = memory();
        let a = follow(&mut b, "a", NavigationTarget::Root);
        b.add_roots(&[url("planted")]).unwrap();
        let c = follow(&mut b, "c", NavigationTarget::Root);
        assert_eq!(b.settle_tree(TreeId(c)).unwrap(), Landing::Restore(a));
    }

    #[test]
    fn settling_a_page_that_is_not_a_root_is_refused() {
        let mut b = memory();
        follow(&mut b, "a", NavigationTarget::Root);
        let child = follow_child(&mut b, "child");
        assert!(b.settle_tree(TreeId(child)).is_err());
    }

    #[test]
    fn a_renamed_tree_keeps_its_name_until_the_name_is_cleared() {
        let mut b = memory();
        let root = follow(&mut b, "a", NavigationTarget::Root);
        b.title_changed(&url("a"), "First title").unwrap();
        let title = |b: &Browser| {
            let states = b.tree_states().unwrap();
            crate::forest::ForestProjection::from_snapshot(b.snapshot().unwrap(), states)
                .unwrap()
                .trees()[0]
                .title
                .clone()
        };
        assert_eq!(title(&b), "First title");

        b.rename_tree(TreeId(root), "  Trip   to Lisbon ").unwrap();
        assert_eq!(title(&b), "Trip to Lisbon");
        // Neither a new page nor a new title changes a chosen name.
        follow_child(&mut b, "b");
        b.title_changed(&url("b"), "Elsewhere").unwrap();
        assert_eq!(title(&b), "Trip to Lisbon");
        assert_eq!(
            b.tree_target(TreeId(root)).unwrap(),
            b.current_id().unwrap().unwrap()
        );

        b.rename_tree(TreeId(root), &"long ".repeat(40)).unwrap();
        assert_eq!(title(&b).chars().count(), MAX_TREE_NAME);
        b.rename_tree(TreeId(root), "   ").unwrap();
        assert_eq!(title(&b), "First title");

        let child = b.current_id().unwrap().unwrap();
        assert!(b.rename_tree(TreeId(child), "Not a tree").is_err());
    }

    #[test]
    fn added_roots_are_trees_in_order_and_load_when_restored() {
        let mut b = memory();
        let earlier = follow(&mut b, "earlier", NavigationTarget::Root);
        let planted = b.add_roots(&[url("first"), url("last")]).unwrap();
        let (first, last) = (planted[0], planted[1]);
        let roots: Vec<_> = b
            .tree_states()
            .unwrap()
            .iter()
            .map(|t| t.root.root())
            .collect();
        assert_eq!(roots, [earlier, first, last]);
        let forest = crate::forest::ForestProjection::from_snapshot(
            b.snapshot().unwrap(),
            b.tree_states().unwrap(),
        )
        .unwrap();
        assert_eq!(forest.trees().len(), 3);
        assert_eq!(forest.current(), Some(earlier));
        let loading = b.restore(last).unwrap();
        b.started(loading.clone()).unwrap();
        assert!(b.committed(url("last-redirected")).unwrap());
        let snapshot = b.snapshot().unwrap();
        assert_eq!(snapshot.current, Some(last));
        assert_eq!(
            snapshot.current_visit().unwrap().url,
            url("last-redirected")
        );
        assert_eq!(b.tree_states().unwrap().len(), 3);
        assert!(b
            .add_roots(&[url("valid"), Url::parse("file:///etc/passwd").unwrap()])
            .is_err());
        assert_eq!(
            b.tree_states().unwrap().len(),
            3,
            "a refused batch plants nothing"
        );
    }

    #[test]
    fn returning_then_following_keeps_both_children() {
        let mut browser = memory();
        let a = follow(&mut browser, "a", NavigationTarget::Root);
        let b = follow_child(&mut browser, "b");
        let load = browser.restore(a).unwrap();
        browser.started(load.clone()).unwrap();
        browser.committed(load).unwrap();
        let c = follow_child(&mut browser, "c");
        let snapshot = browser.snapshot().unwrap();
        assert_eq!(snapshot.visits.len(), 3);
        assert_eq!(snapshot.current, Some(c));
        assert_eq!(
            snapshot
                .visits
                .iter()
                .filter(|v| v.parent == Some(a))
                .map(|v| v.id)
                .collect::<Vec<_>>(),
            vec![b, c]
        );
        assert_eq!(
            snapshot
                .breadcrumb()
                .iter()
                .map(|v| v.id)
                .collect::<Vec<_>>(),
            vec![a, c]
        );
    }

    #[test]
    fn same_url_in_another_context_is_a_new_visit() {
        let mut browser = memory();
        let a = follow(&mut browser, "same", NavigationTarget::Root);
        let b = follow_child(&mut browser, "same");
        assert_ne!(a, b);
        assert_eq!(browser.snapshot().unwrap().visits[1].parent, Some(a));
    }

    #[test]
    fn reload_redirect_and_duplicate_commits_do_not_grow_the_tree() {
        let mut browser = memory();
        let a = follow(&mut browser, "a", NavigationTarget::Root);
        let request = browser.reload().unwrap().unwrap();
        browser.started(request).unwrap();
        browser.redirected(url("final"));
        browser.committed(url("final")).unwrap();
        assert!(!browser.committed(url("final")).unwrap());
        let snapshot = browser.snapshot().unwrap();
        assert_eq!(snapshot.visits.len(), 1);
        assert_eq!(snapshot.current, Some(a));
        assert_eq!(snapshot.visits[0].url, url("final"));
    }

    fn events(browser: &Browser) -> Vec<(VisitId, Option<VisitId>, String)> {
        let mut statement = browser
            .connection
            .prepare("SELECT visit_id, from_visit_id, kind FROM visit_events ORDER BY id")
            .unwrap();
        statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?.parse().unwrap(),
                    row.get::<_, Option<String>>(1)?
                        .map(|id| id.parse().unwrap()),
                    row.get(2)?,
                ))
            })
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }

    #[test]
    fn every_arrival_on_a_page_is_logged_with_the_page_before_it() {
        let mut browser = memory();
        let a = follow(&mut browser, "a", NavigationTarget::Root);
        let b = follow_child(&mut browser, "b");
        let back = browser.back().unwrap().unwrap();
        browser.started(back.clone()).unwrap();
        assert!(browser.committed(back.clone()).unwrap());
        assert!(!browser.committed(back).unwrap());
        let reload = browser.reload().unwrap().unwrap();
        browser.started(reload.clone()).unwrap();
        browser.committed(reload).unwrap();
        let other = follow(&mut browser, "other", NavigationTarget::Root);
        let failed = browser.restore(b).unwrap();
        browser.started(failed.clone()).unwrap();
        browser.failed(&failed, false);
        let event = |visit, from, kind: &str| (visit, from, kind.to_owned());
        assert_eq!(
            events(&browser),
            [
                event(a, None, "new"),
                event(b, Some(a), "new"),
                event(a, Some(b), "return"),
                event(a, Some(a), "reload"),
                event(other, Some(a), "new"),
            ]
        );
    }

    #[test]
    fn failed_and_cancelled_navigation_preserve_committed_state() {
        let mut browser = memory();
        let a = follow(&mut browser, "a", NavigationTarget::Root);
        browser
            .navigate(url("bad"), NavigationTarget::ChildOf(a))
            .unwrap();
        browser.started(url("bad")).unwrap();
        browser.failed(&url("bad"), false);
        assert!(!browser.committed(url("bad")).unwrap());
        browser
            .navigate(url("new"), NavigationTarget::ChildOf(a))
            .unwrap();
        browser.failed(&url("bad"), true);
        browser.started(url("new")).unwrap();
        browser.committed(url("new")).unwrap();
        let snapshot = browser.snapshot().unwrap();
        assert_eq!(snapshot.visits.len(), 2);
        assert_eq!(snapshot.visits[1].parent, Some(a));
    }

    #[test]
    fn reopen_restores_cursor_titles_roots_and_search() {
        let directory = tempfile::tempdir().unwrap();
        let db = directory.path().join("forest.sqlite3");
        let expected = {
            let mut browser = TestBrowser::open(&db).unwrap();
            follow(&mut browser, "a", NavigationTarget::Root);
            browser
                .title_changed(&url("a"), "Rust documentation")
                .unwrap();
            follow(&mut browser, "b", NavigationTarget::Root);
            browser
                .title_changed(&url("b"), "A café page at 100%")
                .unwrap();
            assert_eq!(browser.search("Rust").unwrap().len(), 1);
            assert_eq!(browser.search("%").unwrap().len(), 1);
            assert!(browser
                .snapshot()
                .unwrap()
                .visits
                .iter()
                .all(|v| v.parent.is_none()));
            browser.snapshot().unwrap()
        };
        let browser = TestBrowser::open(&db).unwrap();
        assert_eq!(browser.snapshot().unwrap(), expected);
    }

    #[test]
    fn scroll_positions_survive_reopening_and_only_restores_report_a_target() {
        let directory = tempfile::tempdir().unwrap();
        let db = directory.path().join("forest.sqlite3");
        let (a, b) = {
            let mut browser = TestBrowser::open(&db).unwrap();
            let a = follow(&mut browser, "a", NavigationTarget::Root);
            let b = follow(&mut browser, "b", NavigationTarget::ChildOf(a));
            assert_eq!(browser.scroll(a).unwrap(), None);
            browser.set_scroll(a, 1800).unwrap();
            browser.set_scroll(b, 0).unwrap();
            (a, b)
        };
        let mut browser = TestBrowser::open(&db).unwrap();
        assert_eq!(browser.scroll(a).unwrap(), Some(1800));
        assert_eq!(browser.scroll(b).unwrap(), Some(0));

        assert_eq!(browser.requested_restore(), None);
        browser
            .navigate(url("c"), NavigationTarget::ChildOf(b))
            .unwrap();
        assert_eq!(browser.requested_restore(), None);
        let target = browser.back().unwrap().unwrap();
        assert_eq!(browser.requested_restore(), Some(a));
        browser.started(target).unwrap();
        assert_eq!(browser.requested_restore(), None);
    }

    #[test]
    fn late_title_does_not_rename_a_different_page() {
        let mut browser = memory();
        follow(&mut browser, "a", NavigationTarget::Root);
        follow_child(&mut browser, "b");
        browser.title_changed(&url("a"), "Old title").unwrap();
        assert!(browser
            .snapshot()
            .unwrap()
            .visits
            .iter()
            .all(|v| v.title.is_empty()));
    }

    fn source(comment: &str) -> SourceContext {
        SourceContext {
            page_url: url("thread"),
            citation_url: Some(url(&format!("thread#{comment}"))),
            link_text: format!("Article from {comment}"),
            excerpt: format!("The source comment {comment} mentions a useful article."),
        }
    }

    fn payload(document: &str, text: &str, kind: CaptureKind) -> CapturePayload {
        CapturePayload {
            document_url: url(document),
            citation_url: None,
            title: "A saved article".into(),
            text: text.into(),
            kind,
        }
    }

    #[test]
    fn queueing_keeps_cursor_and_pending_navigation() {
        let mut browser = memory();
        let parent = follow(&mut browser, "thread", NavigationTarget::Root);
        browser
            .navigate(url("other-root"), NavigationTarget::Root)
            .unwrap();
        let children: Vec<_> = (0..3)
            .map(|i| {
                browser
                    .queue_link(parent, url(&format!("article-{i}")), source("comment-one"))
                    .unwrap()
            })
            .collect();
        let snapshot = browser.snapshot().unwrap();
        assert_eq!(snapshot.current, Some(parent));
        assert_eq!(snapshot.visits.len(), 4);
        for child in children {
            let visit = browser.visit(child).unwrap();
            assert_eq!(visit.parent, Some(parent));
            assert_eq!(visit.state, VisitState::Queued);
            assert!(visit.opened_at.is_none());
            assert_eq!(visit.source, Some(source("comment-one")));
        }
        browser.started(url("other-root")).unwrap();
        browser.committed(url("other-root")).unwrap();
        assert!(browser
            .snapshot()
            .unwrap()
            .current_visit()
            .unwrap()
            .parent
            .is_none());
    }

    #[test]
    fn identical_destinations_from_different_comments_remain_distinct() {
        let mut browser = memory();
        let parent = follow(&mut browser, "thread", NavigationTarget::Root);
        let first = browser
            .queue_link(parent, url("article"), source("one"))
            .unwrap();
        let second = browser
            .queue_link(parent, url("article"), source("two"))
            .unwrap();
        assert_ne!(first, second);
        assert_eq!(browser.visit(first).unwrap().source, Some(source("one")));
        assert_eq!(browser.visit(second).unwrap().source, Some(source("two")));
    }

    #[test]
    fn failed_queued_activation_preserves_queue_then_redirect_opens_same_row() {
        let mut browser = memory();
        let parent = follow(&mut browser, "thread", NavigationTarget::Root);
        let queued = browser
            .queue_link(parent, url("article"), source("one"))
            .unwrap();
        let request = browser.restore(queued).unwrap();
        browser.started(request.clone()).unwrap();
        browser.failed(&request, false);
        assert_eq!(browser.snapshot().unwrap().current, Some(parent));
        assert_eq!(browser.visit(queued).unwrap().state, VisitState::Queued);
        assert!(!browser.committed(request.clone()).unwrap());
        browser.restore(queued).unwrap();
        browser.started(request).unwrap();
        browser.redirected(url("final-article"));
        browser.committed(url("final-article")).unwrap();
        let visit = browser.visit(queued).unwrap();
        assert_eq!(browser.snapshot().unwrap().visits.len(), 2);
        assert_eq!(browser.snapshot().unwrap().current, Some(queued));
        assert_eq!(visit.url, url("final-article"));
        assert_eq!(visit.state, VisitState::Opened);
        assert_eq!(visit.source, Some(source("one")));
    }

    #[test]
    fn captures_keep_versions_and_selection_hits_are_individual() {
        let mut browser = memory();
        let visit = follow(&mut browser, "article", NavigationTarget::Root);
        let first = payload(
            "article",
            "The velvet orbital caching phrase is here.",
            CaptureKind::Page,
        );
        let id = browser
            .save_capture(visit, &url("article"), first.clone())
            .unwrap()
            .unwrap();
        assert_eq!(
            browser.save_capture(visit, &url("article"), first).unwrap(),
            Some(id)
        );
        let newer = browser
            .save_capture(
                visit,
                &url("article"),
                payload(
                    "article",
                    "The velvet orbital caching phrase changed today.",
                    CaptureKind::Page,
                ),
            )
            .unwrap()
            .unwrap();
        assert_ne!(newer, id);
        assert!(browser.capture(id).unwrap().text.ends_with("is here."));
        assert_eq!(browser.search("orbital caching").unwrap().len(), 2);
        assert_eq!(browser.search("\"orbital caching\"").unwrap().len(), 2);
        let selection = payload(
            "article",
            "A distinct saved selection",
            CaptureKind::Selection,
        );
        let saved_a = browser
            .save_capture(visit, &url("article"), selection.clone())
            .unwrap()
            .unwrap();
        let saved_b = browser
            .save_capture(visit, &url("article"), selection)
            .unwrap()
            .unwrap();
        assert_ne!(saved_a, saved_b);
        assert_eq!(browser.search("distinct saved selection").unwrap().len(), 2);
        assert_eq!(browser.captures(visit).unwrap().len(), 4);
        for query in ["%", "\"", "a\" OR captures_fts", "[]", "_"] {
            assert!(browser.search(query).is_ok(), "{query}");
        }
    }

    #[test]
    fn capture_search_uses_immutable_source_and_keeps_uncaptured_queue_hits() {
        let mut browser = memory();
        let parent = follow(&mut browser, "thread", NavigationTarget::Root);
        let mut origin = source("one");
        origin.excerpt = "commentcontextneedle from the original source".into();
        let article = browser
            .follow_link(parent, url("article"), origin.clone())
            .unwrap();
        browser.started(article.clone()).unwrap();
        browser.committed(article).unwrap();
        let visit = browser.snapshot().unwrap().current.unwrap();
        let capture = browser
            .save_capture(
                visit,
                &url("article"),
                payload("article", "A searchable article body.", CaptureKind::Page),
            )
            .unwrap()
            .unwrap();
        browser
            .queue_link(parent, url("another"), origin.clone())
            .unwrap();
        let hits = browser.search("commentcontextneedle").unwrap();
        assert_eq!(hits.len(), 2);
        assert!(hits.iter().any(|hit| hit.capture_id == Some(capture)));
        assert!(hits.iter().any(|hit| hit.state == VisitState::Queued));
        let mut changed = origin.clone();
        changed.excerpt = "New DOM context".into();
        assert!(browser.set_source_context(visit, parent, changed).unwrap());
        assert_eq!(browser.capture(capture).unwrap().source, Some(origin));
        let enriched = browser
            .save_capture(
                visit,
                &url("article"),
                payload("article", "A searchable article body.", CaptureKind::Page),
            )
            .unwrap()
            .unwrap();
        assert_ne!(enriched, capture);
        assert_eq!(
            browser.capture(enriched).unwrap().source.unwrap().excerpt,
            "New DOM context"
        );
    }

    #[test]
    fn invalid_or_stale_capture_and_source_results_do_not_mutate_visits() {
        let mut browser = memory();
        let parent = follow(&mut browser, "thread", NavigationTarget::Root);
        let queued = browser
            .queue_link(parent, url("article"), source("one"))
            .unwrap();
        assert!(browser
            .save_capture(
                queued,
                &url("article"),
                payload("article", "not loaded", CaptureKind::Page)
            )
            .unwrap()
            .is_none());
        assert!(!browser
            .set_source_context(queued, queued, source("one"))
            .unwrap());
        let mut wrong_source = source("one");
        wrong_source.page_url = Url::parse("https://other.example/thread").unwrap();
        assert!(browser
            .queue_link(parent, url("article"), wrong_source)
            .is_err());
        let mut wrong_citation = source("one");
        wrong_citation.citation_url = Some(Url::parse("https://other.example/comment").unwrap());
        assert!(browser
            .queue_link(parent, url("article"), wrong_citation)
            .is_err());
        assert!(browser
            .save_capture(
                parent,
                &url("wrong"),
                payload("thread", "stale", CaptureKind::Page)
            )
            .unwrap()
            .is_none());
        let mut foreign = payload("thread", "foreign", CaptureKind::Page);
        foreign.document_url = Url::parse("https://other.example/").unwrap();
        assert!(browser
            .save_capture(parent, &url("thread"), foreign)
            .unwrap()
            .is_none());
        assert!(browser.captures(parent).unwrap().is_empty());
        assert_eq!(browser.snapshot().unwrap().current, Some(parent));
    }

    #[test]
    fn unicode_capture_cap_and_empty_capture_are_safe() {
        let mut browser = memory();
        let visit = follow(&mut browser, "article", NavigationTarget::Root);
        assert!(browser
            .save_capture(
                visit,
                &url("article"),
                payload("article", " \n ", CaptureKind::Page)
            )
            .unwrap()
            .is_none());
        let long = "é".repeat(200_001);
        let id = browser
            .save_capture(
                visit,
                &url("article"),
                payload("article", &long, CaptureKind::Page),
            )
            .unwrap()
            .unwrap();
        assert_eq!(browser.capture(id).unwrap().text.chars().count(), 200_000);
    }

    #[test]
    fn queues_captures_source_and_setting_survive_reopening() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("reading.sqlite3");
        let (snapshot, saved, parent) = {
            let mut browser = TestBrowser::open(&path).unwrap();
            let parent = follow(&mut browser, "thread", NavigationTarget::Root);
            browser
                .queue_link(parent, url("first"), source("one"))
                .unwrap();
            browser
                .queue_link(parent, url("second"), source("one"))
                .unwrap();
            browser.set_capture_enabled(false).unwrap();
            let id = browser
                .save_capture(
                    parent,
                    &url("thread"),
                    payload(
                        "thread",
                        "remembered crimson kestrel",
                        CaptureKind::Selection,
                    ),
                )
                .unwrap()
                .unwrap();
            (
                browser.snapshot().unwrap(),
                browser.capture(id).unwrap(),
                parent,
            )
        };
        let browser = TestBrowser::open(&path).unwrap();
        assert_eq!(browser.snapshot().unwrap(), snapshot);
        assert!(!browser.capture_enabled().unwrap());
        assert_eq!(browser.capture(saved.id).unwrap(), saved);
        let hits = browser.search("crimson kestrel").unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].visit, parent);
        assert_eq!(hits[0].capture_id, Some(saved.id));
    }

    #[test]
    fn source_citation_creates_child_without_replacing_parent_feed_url() {
        let mut browser = memory();
        let parent = follow(&mut browser, "thread", NavigationTarget::Root);
        let mut origin = source("one");
        origin.citation_url = Some(url("exact-comment#one"));
        let child = browser.queue_link(parent, url("article"), origin).unwrap();
        let request = browser.open_citation(child).unwrap().unwrap();
        browser.started(request.clone()).unwrap();
        browser.committed(request).unwrap();
        assert_eq!(browser.visit(parent).unwrap().url, url("thread"));
        assert_eq!(browser.visit(child).unwrap().url, url("article"));
        let snapshot = browser.snapshot().unwrap();
        assert_eq!(snapshot.current_visit().unwrap().parent, Some(parent));
        assert_eq!(
            snapshot.current_visit().unwrap().url,
            url("exact-comment#one")
        );
    }

    #[test]
    fn migration_preserves_handbuilt_mvp1_forest_and_cursor() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("legacy.sqlite3");
        let first = VisitId(Ulid::new());
        let second = VisitId(Ulid::new());
        {
            let connection = Connection::open(&path).unwrap();
            connection.execute_batch("CREATE TABLE visits(id TEXT PRIMARY KEY NOT NULL,parent_id TEXT REFERENCES visits(id),url TEXT NOT NULL,title TEXT NOT NULL DEFAULT '',created_at INTEGER NOT NULL); CREATE TABLE session(singleton INTEGER PRIMARY KEY CHECK(singleton=1),current_id TEXT REFERENCES visits(id));").unwrap();
            connection
                .execute(
                    "INSERT INTO visits VALUES(?1,NULL,?2,'Old root',1234)",
                    params![first.to_string(), url("root").as_str()],
                )
                .unwrap();
            connection
                .execute(
                    "INSERT INTO visits VALUES(?1,?2,?3,'Old child',1235)",
                    params![second.to_string(), first.to_string(), url("child").as_str()],
                )
                .unwrap();
            connection
                .execute("INSERT INTO session VALUES(1,?1)", [second.to_string()])
                .unwrap();
        }
        let expected = {
            let browser = TestBrowser::open(&path).unwrap();
            let snapshot = browser.snapshot().unwrap();
            assert_eq!(snapshot.current, Some(second));
            assert_eq!(snapshot.visits.len(), 2);
            assert!(snapshot
                .visits
                .iter()
                .all(|visit| visit.state == VisitState::Opened));
            assert_eq!(browser.tree_target(TreeId(first)).unwrap(), second);
            assert_eq!(browser.visit(first).unwrap().opened_at, Some(1234));
            assert_eq!(browser.visit(second).unwrap().parent, Some(first));
            assert!(browser.capture_enabled().unwrap());
            assert_eq!(browser.search("Old child").unwrap().len(), 1);
            assert_eq!(
                events(&browser),
                [
                    (first, None, "new".to_owned()),
                    (second, Some(first), "new".to_owned())
                ]
            );
            snapshot
        };
        assert_eq!(
            TestBrowser::open(&path).unwrap().snapshot().unwrap(),
            expected
        );
    }

    #[test]
    fn three_link_exploration_is_searchable_after_restart_with_unopened_siblings() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("exploration.sqlite3");
        let (parent, opened, queued) = {
            let mut browser = TestBrowser::open(&path).unwrap();
            let parent = follow(&mut browser, "thread", NavigationTarget::Root);
            let links: Vec<_> = (0..3)
                .map(|i| {
                    browser
                        .queue_link(parent, url(&format!("article-{i}")), source("comment-one"))
                        .unwrap()
                })
                .collect();
            let request = browser.restore(links[0]).unwrap();
            browser.started(request.clone()).unwrap();
            browser.committed(request).unwrap();
            browser
                .save_capture(
                    links[0],
                    &url("article-0"),
                    payload(
                        "article-0",
                        "A rare violet lighthouse describes the cache policy.",
                        CaptureKind::Page,
                    ),
                )
                .unwrap();
            (parent, links[0], links[1..].to_vec())
        };
        let browser = TestBrowser::open(&path).unwrap();
        let hits = browser.search("violet lighthouse").unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].visit, opened);
        let capture = browser.capture(hits[0].capture_id.unwrap()).unwrap();
        assert!(capture.text.contains("violet lighthouse"));
        assert_eq!(capture.source, Some(source("comment-one")));
        assert_eq!(browser.visit(opened).unwrap().parent, Some(parent));
        for sibling in queued {
            let visit = browser.visit(sibling).unwrap();
            assert_eq!(visit.parent, Some(parent));
            assert_eq!(visit.state, VisitState::Queued);
            assert_eq!(visit.source, Some(source("comment-one")));
        }
    }
    #[test]
    fn tree_targets_survive_queue_failure_selection_and_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("forest.db");
        let (a, child) = {
            let mut browser = TestBrowser::open(&path).unwrap();
            let a = follow(&mut browser, "thread", NavigationTarget::Root);
            let child = follow_child(&mut browser, "child");
            let b = follow(&mut browser, "b", NavigationTarget::Root);
            let queued = browser
                .queue_link(a, url("queued"), source("context"))
                .unwrap();
            assert_eq!(browser.tree_target(TreeId(a)).unwrap(), child);
            let request = browser.restore(queued).unwrap();
            browser.started(request.clone()).unwrap();
            browser.failed(&request, false);
            assert_eq!(browser.tree_target(TreeId(a)).unwrap(), child);
            assert_eq!(browser.snapshot().unwrap().current, Some(b));
            let request = browser
                .restore(browser.tree_target(TreeId(a)).unwrap())
                .unwrap();
            browser.started(request.clone()).unwrap();
            browser.committed(request).unwrap();
            assert_eq!(browser.snapshot().unwrap().current, Some(child));
            (a, child)
        };
        let browser = TestBrowser::open(&path).unwrap();
        assert_eq!(browser.tree_target(TreeId(a)).unwrap(), child);
        assert_eq!(browser.snapshot().unwrap().current, Some(child));
    }

    #[test]
    fn migration_from_v1_initializes_targets_and_preserves_theme_on_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("v1.db");
        let a = VisitId(Ulid::new());
        let child = VisitId(Ulid::new());
        let b = VisitId(Ulid::new());
        {
            let connection = Connection::open(&path).unwrap();
            connection.execute_batch("CREATE TABLE visits(id TEXT PRIMARY KEY NOT NULL,
                parent_id TEXT REFERENCES visits(id), url TEXT NOT NULL, title TEXT NOT NULL DEFAULT '',
                created_at INTEGER NOT NULL, opened_at INTEGER);
                CREATE TABLE session(singleton INTEGER PRIMARY KEY CHECK(singleton=1),
                current_id TEXT REFERENCES visits(id), capture_enabled INTEGER NOT NULL DEFAULT 1);
                PRAGMA user_version=1;").unwrap();
            for (id, parent) in [(a, None), (child, Some(a)), (b, None)] {
                connection
                    .execute(
                        "INSERT INTO visits VALUES(?1,?2,'https://example.org','',1,1)",
                        params![id.to_string(), parent.map(|id| id.to_string())],
                    )
                    .unwrap();
            }
            connection
                .execute("INSERT INTO session VALUES(1,?1,1)", [child.to_string()])
                .unwrap();
        }
        let expected = {
            let browser = TestBrowser::open(&path).unwrap();
            assert_eq!(browser.tree_target(TreeId(a)).unwrap(), child);
            assert_eq!(browser.tree_target(TreeId(b)).unwrap(), b);
            assert_eq!(browser.theme_mode().unwrap(), ThemeMode::Dark);
            browser
                .connection
                .execute("UPDATE session SET theme_mode='system'", [])
                .unwrap();
            assert!(browser
                .connection
                .execute("UPDATE session SET theme_mode='unknown'", [])
                .is_err());
            assert!(browser
                .connection
                .execute(
                    "UPDATE tree_state SET last_visit_id=?1 WHERE root_id=?2",
                    params![b.to_string(), a.to_string()]
                )
                .is_err());
            assert!(browser
                .connection
                .execute(
                    "INSERT INTO tree_state(root_id, last_visit_id) VALUES(?1,?1)",
                    [child.to_string()]
                )
                .is_err());
            browser.tree_states().unwrap()
        };
        let browser = TestBrowser::open(&path).unwrap();
        assert_eq!(browser.tree_states().unwrap(), expected);
        assert_eq!(
            browser.theme_mode().unwrap(),
            ThemeMode::System,
            "the dark migration runs once; a later System choice survives reopening"
        );
    }
    #[test]
    fn explicit_parent_is_validated_and_survives_cursor_changes_before_start() {
        let mut browser = memory();
        let a = follow(&mut browser, "thread", NavigationTarget::Root);
        let b = follow(&mut browser, "b", NavigationTarget::Root);
        let queued = browser
            .queue_link(a, url("queued"), source("context"))
            .unwrap();
        let missing = VisitId(Ulid::new());
        for parent in [queued, missing] {
            assert!(browser
                .navigate(url("bad"), NavigationTarget::ChildOf(parent))
                .is_err());
        }
        let request = browser
            .navigate(url("anchored"), NavigationTarget::ChildOf(a))
            .unwrap();
        browser
            .connection
            .execute("UPDATE session SET current_id=?1", [b.to_string()])
            .unwrap();
        browser.started(request.clone()).unwrap();
        browser.committed(request).unwrap();
        assert_eq!(
            browser.snapshot().unwrap().current_visit().unwrap().parent,
            Some(a)
        );
    }
    #[test]
    fn preview_is_bounded_prefers_source_and_uses_latest_capture() {
        let mut browser = memory();
        let root = follow(&mut browser, "thread", NavigationTarget::Root);
        assert_eq!(browser.preview(root).unwrap().snippet, "");
        browser
            .save_capture(
                root,
                &url("thread"),
                payload("thread", "old capture", CaptureKind::Page),
            )
            .unwrap();
        let body = "界".repeat(200_000);
        browser
            .save_capture(
                root,
                &url("thread"),
                payload("thread", &body, CaptureKind::Page),
            )
            .unwrap();
        assert_eq!(browser.preview(root).unwrap().snippet, "界".repeat(320));
        let queued = browser
            .queue_link(root, url("article"), source("context"))
            .unwrap();
        let excerpt = browser.visit(queued).unwrap().source.unwrap().excerpt;
        assert_eq!(browser.preview(queued).unwrap().snippet, excerpt);
        let request = browser.restore(queued).unwrap();
        browser.started(request.clone()).unwrap();
        browser.committed(request).unwrap();
        browser
            .save_capture(
                queued,
                &url("article"),
                payload(
                    "article",
                    "capture must not replace source",
                    CaptureKind::Page,
                ),
            )
            .unwrap();
        assert_eq!(browser.preview(queued).unwrap().snippet, excerpt);
        assert!(browser.preview(VisitId(Ulid::new())).is_err());
    }

    #[test]
    fn cursor_insert_and_tree_target_roll_back_together() {
        let mut browser = memory();
        let a = follow(&mut browser, "thread", NavigationTarget::Root);
        let queued = browser
            .queue_link(a, url("queued"), source("context"))
            .unwrap();
        let before = browser.snapshot().unwrap();
        let states = browser.tree_states().unwrap();
        assert!(browser
            .connection
            .execute(
                "UPDATE tree_state SET last_visit_id=?1 WHERE root_id=?2",
                params![queued.to_string(), a.to_string()]
            )
            .is_err());
        browser
            .connection
            .execute_batch(
                "CREATE TRIGGER reject_target BEFORE UPDATE ON tree_state
            BEGIN SELECT RAISE(ABORT, 'test failure'); END;",
            )
            .unwrap();
        let request = browser
            .navigate(url("child"), NavigationTarget::ChildOf(a))
            .unwrap();
        browser.started(request.clone()).unwrap();
        assert!(browser.committed(request).is_err());
        let after = browser.snapshot().unwrap();
        assert_eq!(after, before);
        assert_eq!(browser.tree_states().unwrap(), states);
    }

    #[test]
    fn imported_pages_are_searchable_and_replaceable() {
        let dir = tempfile::tempdir().unwrap();
        let mut browser = TestBrowser::open(&dir.path().join("forest.sqlite3")).unwrap();
        let page = |url: &str, title: &str, visits| ImportedPage {
            url: Url::parse(url).unwrap(),
            title: title.into(),
            visits,
            typed: 0,
            last_visit: Some(1),
        };
        browser
            .replace_imported(
                "chrome:Default",
                &[
                    page("https://docs.rs/wry", "wry 100% docs", 3),
                    page("https://docs.rs/tao", "tao docs", 9),
                    page("https://servo.org/", "Servo", 0),
                ],
            )
            .unwrap();
        let hits = browser.search_imported("docs", 10).unwrap();
        assert_eq!(
            hits.iter().map(|p| p.url.path()).collect::<Vec<_>>(),
            ["/tao", "/wry"],
            "the most visited first"
        );
        assert_eq!(
            browser.search_imported("100%", 10).unwrap().len(),
            1,
            "LIKE wildcards are literal"
        );
        assert_eq!(
            browser.search_imported("o", 10).unwrap()[0].url.host_str(),
            Some("docs.rs"),
            "history stays ordered by visits"
        );
        assert!(
            browser.snapshot().unwrap().visits.is_empty(),
            "imports plant no trees"
        );
        browser
            .replace_imported("chrome:Default", &[page("https://servo.org/", "Servo", 1)])
            .unwrap();
        assert!(
            browser.search_imported("docs", 10).unwrap().is_empty(),
            "a re-import replaces its source"
        );
    }

    #[test]
    fn a_download_belongs_to_the_page_it_started_from() {
        let mut b = memory();
        assert_eq!(b.download_page(Some(&url("file"))).unwrap(), None);
        // A file address typed with no page open has no page to belong to.
        let file = b.navigate(url("file"), NavigationTarget::Root).unwrap();
        b.started(file.clone()).unwrap();
        assert_eq!(b.download_page(Some(&file)).unwrap(), None);
        b.failed(&file, false);

        let root = follow(&mut b, "a", NavigationTarget::Root);
        let page = follow_child(&mut b, "b");
        // Saved from the page on screen: a context menu, a script, a link marked `download`.
        assert_eq!(b.download_page(Some(&url("pic.png"))).unwrap(), Some(page));
        assert_eq!(b.download_page(None).unwrap(), Some(page));

        // A link followed from another page than the one on screen, which turns out to be
        // a file: the file belongs to the page the link was followed from.
        let file = b
            .navigate(url("file"), NavigationTarget::ChildOf(root))
            .unwrap();
        assert_eq!(b.download_page(Some(&file)).unwrap(), Some(page));
        b.started(file.clone()).unwrap();
        assert_eq!(b.download_page(Some(&file)).unwrap(), Some(root));
        // Meanwhile the page on screen still owns what it saves itself.
        assert_eq!(b.download_page(Some(&url("pic.png"))).unwrap(), Some(page));
        b.redirected(url("cdn/file"));
        assert_eq!(b.download_page(Some(&url("cdn/file"))).unwrap(), Some(root));
        b.failed(&url("cdn/file"), false);
        assert_eq!(b.download_page(Some(&url("cdn/file"))).unwrap(), Some(page));

        // A pending link that turns out to be a file: the file goes to the page the link
        // is on, whichever page is on screen when it is opened.
        let queued = b.queue_link(root, url("paper.pdf"), source("one")).unwrap();
        let request = b.restore(queued).unwrap();
        b.started(request.clone()).unwrap();
        assert_eq!(b.download_page(Some(&request)).unwrap(), Some(root));
        b.failed(&request, false);

        // A file address opened as a tree of its own came from no page, typed or handed
        // over by another app: its file falls back to the page on screen.
        let file = b.navigate(url("file"), NavigationTarget::Root).unwrap();
        b.started(file.clone()).unwrap();
        assert_eq!(b.download_page(Some(&file)).unwrap(), Some(page));
        b.failed(&file, false);
        let planted = b.add_roots(&[url("file")]).unwrap()[0];
        let request = b.restore(planted).unwrap();
        b.started(request.clone()).unwrap();
        assert_eq!(b.download_page(Some(&request)).unwrap(), Some(page));
        assert_eq!(b.snapshot().unwrap().current, Some(page));
    }

    #[test]
    fn downloads_stay_on_their_page_in_order_and_survive_reopening() {
        let directory = tempfile::tempdir().unwrap();
        let db = directory.path().join("forest.sqlite3");
        let (root, page, expected) = {
            let mut b = TestBrowser::open(&db).unwrap();
            let root = follow(&mut b, "a", NavigationTarget::Root);
            let page = follow_child(&mut b, "b");
            let before = b.snapshot().unwrap();
            let report = b
                .record_download(
                    page,
                    Path::new("/home/me/Downloads/report.pdf"),
                    1_234_567,
                    Some(&url("files/report.pdf")),
                )
                .unwrap();
            // A file a script built in the page has no web address.
            let export = b
                .record_download(
                    root,
                    Path::new("/home/me/Downloads/Résumé (1).csv"),
                    0,
                    None,
                )
                .unwrap();
            let again = b
                .record_download(
                    page,
                    Path::new("/home/me/Downloads/report (1).pdf"),
                    12,
                    None,
                )
                .unwrap();
            assert_eq!(
                b.snapshot().unwrap(),
                before,
                "a download is no page and moves no cursor"
            );
            let downloads = b.downloads().unwrap();
            assert_eq!(
                downloads.iter().map(|d| d.id).collect::<Vec<_>>(),
                [report, export, again]
            );
            assert_eq!(
                downloads.iter().map(Download::name).collect::<Vec<_>>(),
                ["report.pdf", "Résumé (1).csv", "report (1).pdf"]
            );
            assert_eq!(downloads[0].visit, page);
            assert_eq!(downloads[0].size, 1_234_567);
            assert_eq!(downloads[0].source, Some(url("files/report.pdf")));
            assert_eq!(downloads[1].visit, root);
            assert_eq!(downloads[1].source, None);
            assert_eq!(b.download(again).unwrap(), downloads[2]);
            (root, page, downloads)
        };
        let mut b = TestBrowser::open(&db).unwrap();
        assert_eq!(b.downloads().unwrap(), expected);

        let unknown: VisitId = Ulid::new().to_string().parse().unwrap();
        assert!(b
            .record_download(unknown, Path::new("/tmp/x"), 1, None)
            .is_err());
        assert!(b.download(expected[2].id + 1).is_err());
        use std::os::unix::ffi::OsStrExt;
        let odd = Path::new(std::ffi::OsStr::from_bytes(b"/tmp/\xff"));
        assert!(b.record_download(page, odd, 1, None).is_err());
        assert_eq!(b.downloads().unwrap(), expected);

        // A page that goes takes its downloads' records with it, never the files.
        b.connection
            .execute("DELETE FROM visits WHERE id=?1", [page.to_string()])
            .unwrap_err();
        b.connection
            .execute_batch("DELETE FROM tree_state; UPDATE session SET current_id=NULL;")
            .unwrap();
        b.connection
            .execute("DELETE FROM visits WHERE id=?1", [page.to_string()])
            .unwrap();
        let left = b.downloads().unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].visit, root);
    }

    #[test]
    fn a_forest_from_before_downloads_gains_the_table_and_keeps_its_version() {
        let directory = tempfile::tempdir().unwrap();
        let db = directory.path().join("forest.sqlite3");
        let version = |connection: &Connection| -> i64 {
            connection
                .pragma_query_value(None, "user_version", |row| row.get(0))
                .unwrap()
        };
        let (page, before) = {
            let mut b = TestBrowser::open(&db).unwrap();
            let page = follow(&mut b, "a", NavigationTarget::Root);
            b.connection.execute_batch("DROP TABLE downloads").unwrap();
            (page, version(&b.connection))
        };
        let mut b = TestBrowser::open(&db).unwrap();
        assert_eq!(version(&b.connection), before);
        assert!(b.downloads().unwrap().is_empty());
        b.record_download(page, Path::new("/tmp/report.pdf"), 1, None)
            .unwrap();
        assert_eq!(b.downloads().unwrap().len(), 1);
    }
}
