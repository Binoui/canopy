use super::*;
use crate::address::WebOrigin;

const STEPS: &[u16] = &[
    25, 33, 50, 67, 75, 80, 90, 100, 110, 125, 150, 175, 200, 250, 300, 400, 500,
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PageScale(u8);

impl Default for PageScale {
    fn default() -> Self {
        Self::from_stored_percent(100).expect("100 is a page scale step")
    }
}

impl PageScale {
    pub fn percent(self) -> u16 {
        STEPS[usize::from(self.0)]
    }

    pub(crate) fn from_stored_percent(percent: i64) -> Result<Self> {
        STEPS
            .iter()
            .position(|&step| i64::from(step) == percent)
            .map(|index| Self(index as u8))
            .context("Invalid stored page scale.")
    }

    pub(crate) fn larger(self) -> Self {
        Self((self.0 + 1).min(STEPS.len() as u8 - 1))
    }

    pub(crate) fn smaller(self) -> Self {
        Self(self.0.saturating_sub(1))
    }
}

impl Browser {
    pub fn page_scale(&self, origin: &WebOrigin) -> Result<PageScale> {
        let percent: Option<i64> = self
            .connection
            .query_row(
                "SELECT percent FROM site_page_scale WHERE origin = ?1",
                [origin.as_str()],
                |row| row.get(0),
            )
            .optional()?;
        percent
            .map(PageScale::from_stored_percent)
            .transpose()
            .map(|level| level.unwrap_or_default())
    }

    pub fn set_page_scale(&self, origin: &WebOrigin, level: PageScale) -> Result<()> {
        if level == PageScale::default() {
            self.connection.execute(
                "DELETE FROM site_page_scale WHERE origin = ?1",
                [origin.as_str()],
            )?;
        } else {
            self.connection.execute(
                "INSERT INTO site_page_scale(origin, percent) VALUES(?1, ?2)
                 ON CONFLICT(origin) DO UPDATE SET percent = excluded.percent",
                params![origin.as_str(), level.percent()],
            )?;
        }
        Ok(())
    }
}

pub(super) fn migrate(tx: &rusqlite::Transaction<'_>) -> Result<()> {
    tx.execute_batch(
        "CREATE TABLE site_page_scale (
            origin TEXT PRIMARY KEY NOT NULL,
            percent INTEGER NOT NULL CHECK(percent BETWEEN 25 AND 500)
         );
         PRAGMA user_version = 13;",
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_clamp_and_round_trip() {
        let mut level = PageScale::from_stored_percent(25).unwrap();
        assert_eq!(level.smaller(), level);
        for &percent in STEPS {
            assert_eq!(level.percent(), percent);
            assert_eq!(
                PageScale::from_stored_percent(i64::from(percent)).unwrap(),
                level
            );
            level = level.larger();
        }
        assert_eq!(level.larger(), level);
        assert_eq!(PageScale::default().percent(), 100);
        for invalid in [-1, 0, 26, 99, 101, 501, i64::MAX] {
            assert!(PageScale::from_stored_percent(invalid).is_err());
        }
    }

    #[test]
    fn migrate_v12_preserves_visits_and_origin_policy_survives_reopen() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("forest.sqlite3");
        let browser = Browser::open(&path)?;
        let visit = Ulid::new().to_string();
        browser.connection.execute("INSERT INTO visits(id,url,title,created_at,opened_at) VALUES(?1,'https://example.org/a','Kept',1,1)", [&visit])?;
        browser
            .connection
            .execute_batch("DROP TABLE site_page_scale; PRAGMA user_version = 12;")?;
        drop(browser);
        let browser = Browser::open(&path)?;
        assert_eq!(
            browser
                .connection
                .pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))?,
            13
        );
        assert_eq!(
            browser.connection.query_row(
                "SELECT title FROM visits WHERE id=?1",
                [&visit],
                |r| r.get::<_, String>(0)
            )?,
            "Kept"
        );
        let origin = |url| WebOrigin::of(&url::Url::parse(url).unwrap()).unwrap();
        let site = origin("https://example.org/a");
        let level = PageScale::default().larger().larger().larger();
        browser.set_page_scale(&site, level)?;
        assert_eq!(browser.page_scale(&origin("https://example.org/b"))?, level);
        for url in ["http://example.org/a", "https://example.org:444/a"] {
            assert_eq!(browser.page_scale(&origin(url))?, PageScale::default());
        }
        drop(browser);
        let browser = Browser::open(&path)?;
        assert_eq!(browser.page_scale(&site)?, level);
        browser.set_page_scale(&site, PageScale::default())?;
        assert_eq!(
            browser
                .connection
                .query_row("SELECT COUNT(*) FROM site_page_scale", [], |r| r
                    .get::<_, i64>(0))?,
            0
        );
        drop(browser);
        assert_eq!(
            Browser::open(&path)?.page_scale(&site)?,
            PageScale::default()
        );
        Ok(())
    }

    #[test]
    fn refuses_v14_and_invalid_stored_steps() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("forest.sqlite3");
        let browser = Browser::open(&path)?;
        let site = WebOrigin::of(&url::Url::parse("https://example.org")?).unwrap();
        browser.connection.execute(
            "INSERT INTO site_page_scale VALUES(?1, 101)",
            [site.as_str()],
        )?;
        assert!(browser.page_scale(&site).is_err());
        browser.connection.pragma_update(None, "user_version", 14)?;
        drop(browser);
        assert!(Browser::open(&path)
            .err()
            .unwrap()
            .to_string()
            .contains("newer version"));
        Ok(())
    }
}
