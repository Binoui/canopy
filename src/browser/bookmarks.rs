use super::{now_ms, Browser, ImportedPage};
use anyhow::Result;
use rusqlite::{params, OptionalExtension, Transaction};
use url::Url;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct BookmarkId(i64);
impl BookmarkId {
    pub fn get(self) -> i64 {
        self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Bookmark {
    pub id: BookmarkId,
    pub url: Url,
    pub title: String,
    pub folder: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImportedBookmark {
    pub url: Url,
    pub title: String,
    pub folder: String,
    pub visits: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BookmarkToggle {
    Added,
    Removed,
    NoPage,
}

impl Browser {
    pub fn bookmark(&mut self, url: &Url, title: &str) -> Result<()> {
        let tx = self.connection.transaction()?;
        append(&tx, url, title, None, now_ms()?)?;
        tx.commit()?;
        Ok(())
    }

    pub fn bookmarks(&self) -> Result<Vec<Bookmark>> {
        let mut stmt = self.connection.prepare(
            "SELECT id,url,title,folder FROM bookmarks
             ORDER BY folder IS NOT NULL,folder,position,id",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get(2)?,
                r.get(3)?,
            ))
        })?;
        rows.map(|row| {
            let (id, url, title, folder) = row?;
            Ok(Bookmark {
                id: BookmarkId(id),
                url: Url::parse(&url)?,
                title,
                folder,
            })
        })
        .collect()
    }

    pub fn toggle_current_bookmark(&mut self) -> Result<BookmarkToggle> {
        let tx = self.connection.transaction()?;
        let page = tx
            .query_row(
                "SELECT v.url,v.title FROM session s JOIN visits v ON v.id=s.current_id",
                [],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
            )
            .optional()?;
        let change = if let Some((url, title)) = page {
            if tx.execute("DELETE FROM bookmarks WHERE url=?1", [&url])? > 0 {
                BookmarkToggle::Removed
            } else {
                append(&tx, &Url::parse(&url)?, &title, None, now_ms()?)?;
                BookmarkToggle::Added
            }
        } else {
            BookmarkToggle::NoPage
        };
        tx.commit()?;
        Ok(change)
    }

    pub fn remove_bookmark(&mut self, id: BookmarkId) -> Result<()> {
        self.connection
            .execute("DELETE FROM bookmarks WHERE id=?1", [id.0])?;
        Ok(())
    }

    pub fn import_pages(
        &mut self,
        source: &str,
        pages: &[ImportedPage],
        bookmarks: &[ImportedBookmark],
    ) -> Result<usize> {
        let tx = self.connection.transaction()?;
        tx.execute("DELETE FROM imported_pages WHERE source=?1", [source])?;
        for page in pages {
            tx.execute("INSERT INTO imported_pages(source,url,title,visits,typed,last_visit) VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(source,url) DO UPDATE SET title=excluded.title,visits=excluded.visits,typed=excluded.typed,last_visit=excluded.last_visit", params![source,page.url.as_str(),page.title,page.visits as i64,page.typed as i64,page.last_visit])?;
        }
        let added = append_imported(&tx, bookmarks, now_ms()?)?;
        tx.commit()?;
        Ok(added)
    }
}

fn folder_from_chrome(path: &str) -> Option<String> {
    let path = if path == "Bookmarks bar" {
        ""
    } else {
        path.strip_prefix("Bookmarks bar › ").unwrap_or(path)
    };
    (!path.trim().is_empty()).then(|| path.to_owned())
}

fn append_imported(
    tx: &Transaction<'_>,
    bookmarks: &[ImportedBookmark],
    now: i64,
) -> Result<usize> {
    let mut ordered: Vec<_> = bookmarks.iter().collect();
    ordered.sort_by_key(|b| (folder_from_chrome(&b.folder), std::cmp::Reverse(b.visits)));
    let mut added = 0;
    for b in ordered {
        added += usize::from(append(
            tx,
            &b.url,
            &b.title,
            folder_from_chrome(&b.folder).as_deref(),
            now,
        )?);
    }
    Ok(added)
}

fn append(
    tx: &Transaction<'_>,
    url: &Url,
    title: &str,
    folder: Option<&str>,
    now: i64,
) -> Result<bool> {
    Ok(tx.execute("INSERT INTO bookmarks(url,title,folder,position,added_at) SELECT ?1,?2,?3,COALESCE(MAX(position)+1,0),?4 FROM bookmarks WHERE folder IS ?3 ON CONFLICT(url) DO NOTHING", params![url.as_str(),title,folder,now])? > 0)
}

pub(super) fn migrate(tx: &Transaction<'_>) -> Result<()> {
    tx.execute_batch(
        "CREATE TABLE bookmarks (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            url TEXT NOT NULL UNIQUE,
            title TEXT NOT NULL,
            folder TEXT CHECK(folder IS NULL OR length(trim(folder)) > 0),
            position INTEGER NOT NULL CHECK(position >= 0),
            added_at INTEGER NOT NULL
        );
        CREATE UNIQUE INDEX bookmarks_folder_position ON bookmarks(COALESCE(folder,''),position);",
    )?;
    let mut stmt = tx.prepare("SELECT url,MAX(title),MAX(folder),SUM(visits) FROM imported_pages WHERE folder IS NOT NULL GROUP BY url ORDER BY url")?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, i64>(3)?,
        ))
    })?;
    let mut old = Vec::new();
    for row in rows {
        let (url, title, folder, visits) = row?;
        if let Ok(url) = crate::address::web_url(&url) {
            old.push(ImportedBookmark {
                url,
                title,
                folder,
                visits: visits.max(0) as usize,
            });
        }
    }
    drop(stmt);
    append_imported(tx, &old, now_ms()?)?;
    tx.execute_batch("DELETE FROM imported_pages WHERE folder IS NOT NULL AND visits=0 AND typed=0 AND last_visit IS NULL; ALTER TABLE imported_pages DROP COLUMN folder; PRAGMA user_version=10;")?;
    Ok(())
}
