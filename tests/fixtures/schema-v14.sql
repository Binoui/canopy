CREATE TABLE visits (
                 id TEXT PRIMARY KEY NOT NULL,
                 parent_id TEXT REFERENCES visits(id),
                 url TEXT NOT NULL,
                 title TEXT NOT NULL DEFAULT '',
                 created_at INTEGER NOT NULL
             , opened_at INTEGER, scroll_y INTEGER, dismissed_at INTEGER, branch_settled_at INTEGER);
CREATE INDEX visits_parent ON visits(parent_id);
CREATE TABLE downloads (
                 id INTEGER PRIMARY KEY,
                 visit_id TEXT NOT NULL REFERENCES visits(id) ON DELETE CASCADE,
                 path TEXT NOT NULL,
                 size INTEGER NOT NULL,
                 source_url TEXT,
                 saved_at INTEGER NOT NULL
             );
CREATE INDEX downloads_visit ON downloads(visit_id, id);
CREATE TABLE session (
                 singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
                 current_id TEXT REFERENCES visits(id)
             , capture_enabled INTEGER NOT NULL DEFAULT 1, theme_mode TEXT NOT NULL DEFAULT 'system'
                    CHECK(theme_mode IN ('system', 'light', 'dark')), trees_help INTEGER NOT NULL DEFAULT 1);
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
CREATE TABLE tree_state (
                    root_id TEXT PRIMARY KEY NOT NULL REFERENCES visits(id),
                    last_visit_id TEXT NOT NULL REFERENCES visits(id)
                 , name TEXT, settled_at INTEGER CHECK(settled_at > 0), pin_order INTEGER CHECK(pin_order > 0));
CREATE TRIGGER tree_state_validate_INSERT BEFORE INSERT ON tree_state
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
                     END;
CREATE TRIGGER tree_state_validate_UPDATE BEFORE UPDATE ON tree_state
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
                     END;
CREATE TABLE imported_pages (
                    source TEXT NOT NULL,
                    url TEXT NOT NULL,
                    title TEXT NOT NULL,
                    visits INTEGER NOT NULL,
                    typed INTEGER NOT NULL,
                    last_visit INTEGER,
                    PRIMARY KEY(source, url)
                 );
CREATE TABLE visit_events (
                    id INTEGER PRIMARY KEY,
                    visit_id TEXT NOT NULL REFERENCES visits(id) ON DELETE CASCADE,
                    from_visit_id TEXT REFERENCES visits(id) ON DELETE SET NULL,
                    kind TEXT NOT NULL CHECK(kind IN ('new', 'return', 'reload')),
                    at INTEGER NOT NULL
                 );
CREATE INDEX visit_events_visit ON visit_events(visit_id, at);
CREATE INDEX visit_events_at ON visit_events(at);
CREATE UNIQUE INDEX tree_state_pin_order ON tree_state(pin_order)
                    WHERE pin_order IS NOT NULL;
CREATE TABLE bookmarks (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            url TEXT NOT NULL UNIQUE,
            title TEXT NOT NULL,
            folder TEXT CHECK(folder IS NULL OR length(trim(folder)) > 0),
            position INTEGER NOT NULL CHECK(position >= 0),
            added_at INTEGER NOT NULL
        );
CREATE UNIQUE INDEX bookmarks_folder_position ON bookmarks(COALESCE(folder,''),position);
CREATE TABLE declined_suggestions(key TEXT PRIMARY KEY NOT NULL);
CREATE TABLE site_page_scale (
            origin TEXT PRIMARY KEY NOT NULL,
            percent INTEGER NOT NULL CHECK(percent BETWEEN 25 AND 500)
         );
INSERT INTO session(singleton) VALUES(1);
PRAGMA user_version=14;
