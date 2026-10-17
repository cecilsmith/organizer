//! Local SQLite store: configuration plus a cache of every synced item, so the
//! app opens instantly with the last known state and works offline.

use std::path::Path;
use std::sync::Mutex;

use rusqlite::{Connection, OptionalExtension, params};

use crate::categorize::default_categories;
use crate::error::Result;
use crate::model::*;

pub struct Db {
    conn: Mutex<Connection>,
}

#[derive(Clone, Copy)]
enum ListKind {
    Reminders,
    Events,
}

impl ListKind {
    fn table(self) -> &'static str {
        match self {
            ListKind::Reminders => "reminder_lists",
            ListKind::Events => "event_calendars",
        }
    }
}

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS categories (
    id TEXT PRIMARY KEY, name TEXT NOT NULL, color TEXT NOT NULL,
    label TEXT NOT NULL, position INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS accounts (
    id TEXT PRIMARY KEY, login TEXT NOT NULL, host TEXT NOT NULL,
    name TEXT, avatar_url TEXT, scopes TEXT NOT NULL DEFAULT '',
    token_source TEXT NOT NULL DEFAULT 'keychain'
);
CREATE TABLE IF NOT EXISTS repos (
    id TEXT PRIMARY KEY, account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    owner TEXT NOT NULL, name TEXT NOT NULL, url TEXT NOT NULL, default_category TEXT
);
CREATE TABLE IF NOT EXISTS projects (
    id TEXT PRIMARY KEY, account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    owner TEXT NOT NULL, number INTEGER NOT NULL, title TEXT NOT NULL, url TEXT NOT NULL,
    default_category TEXT
);
CREATE TABLE IF NOT EXISTS reminder_lists (
    id TEXT PRIMARY KEY, title TEXT NOT NULL, color TEXT, source_title TEXT,
    writable INTEGER NOT NULL DEFAULT 1, enabled INTEGER NOT NULL DEFAULT 1,
    default_category TEXT, present INTEGER NOT NULL DEFAULT 1
);
-- One row per (source, item). The same GitHub issue can be cached by both its
-- repo and a project; they are merged when building a snapshot.
CREATE TABLE IF NOT EXISTS items (
    source_id TEXT NOT NULL, id TEXT NOT NULL, data TEXT NOT NULL,
    PRIMARY KEY (source_id, id)
);
-- Calendars that hold events; same shape as reminder_lists.
CREATE TABLE IF NOT EXISTS event_calendars (
    id TEXT PRIMARY KEY, title TEXT NOT NULL, color TEXT, source_title TEXT,
    writable INTEGER NOT NULL DEFAULT 1, enabled INTEGER NOT NULL DEFAULT 1,
    default_category TEXT, present INTEGER NOT NULL DEFAULT 1
);
CREATE TABLE IF NOT EXISTS events (
    id TEXT PRIMARY KEY, data TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS sync_state (
    source_id TEXT PRIMARY KEY, last_synced TEXT, error TEXT
);
"#;

/// Adds columns introduced after a database was first created.
fn migrate(conn: &Connection) -> Result<()> {
    let has_column = |table: &str, column: &str| -> Result<bool> {
        let mut st = conn.prepare(&format!("SELECT 1 FROM pragma_table_info('{table}') WHERE name = ?1"))?;
        Ok(st.exists([column])?)
    };
    for table in ["accounts", "repos"] {
        if !has_column(table, "provider")? {
            conn.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN provider TEXT NOT NULL DEFAULT 'github'"))?;
        }
    }
    // Accounts linked before CLI tokens were kept out of the Keychain.
    if !has_column("accounts", "token_source")? {
        conn.execute_batch("ALTER TABLE accounts ADD COLUMN token_source TEXT NOT NULL DEFAULT 'keychain'")?;
    }
    Ok(())
}

impl Db {
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL; PRAGMA foreign_keys = ON;",
        )?;
        conn.execute_batch(SCHEMA)?;
        migrate(&conn)?;
        let db = Self { conn: Mutex::new(conn) };
        db.seed()?;
        Ok(db)
    }

    #[cfg(test)]
    pub fn in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        conn.execute_batch(SCHEMA)?;
        migrate(&conn)?;
        let db = Self { conn: Mutex::new(conn) };
        db.seed()?;
        Ok(db)
    }

    fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn seed(&self) -> Result<()> {
        let empty: bool =
            self.conn().query_row("SELECT COUNT(*) = 0 FROM categories", [], |r| r.get(0))?;
        if empty {
            for c in default_categories() {
                self.upsert_category(&c)?;
            }
        }
        Ok(())
    }

    // ---- categories -------------------------------------------------------

    pub fn categories(&self) -> Result<Vec<Category>> {
        let conn = self.conn();
        let mut st = conn.prepare_cached(
            "SELECT id, name, color, label, position FROM categories ORDER BY position, name",
        )?;
        let rows = st.query_map([], |r| {
            Ok(Category {
                id: r.get(0)?,
                name: r.get(1)?,
                color: r.get(2)?,
                label: r.get(3)?,
                position: r.get(4)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn upsert_category(&self, c: &Category) -> Result<()> {
        self.conn().execute(
            "INSERT INTO categories (id, name, color, label, position) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(id) DO UPDATE SET name = ?2, color = ?3, label = ?4, position = ?5",
            params![c.id, c.name, c.color, c.label, c.position],
        )?;
        Ok(())
    }

    pub fn delete_category(&self, id: &str) -> Result<()> {
        let conn = self.conn();
        conn.execute("DELETE FROM categories WHERE id = ?1", [id])?;
        for table in ["repos", "projects", "reminder_lists", "event_calendars"] {
            conn.execute(
                &format!("UPDATE {table} SET default_category = NULL WHERE default_category = ?1"),
                [id],
            )?;
        }
        Ok(())
    }

    // ---- accounts ---------------------------------------------------------

    pub fn accounts(&self) -> Result<Vec<Account>> {
        let conn = self.conn();
        let mut st = conn.prepare_cached(
            "SELECT id, login, host, name, avatar_url, scopes, provider, token_source
             FROM accounts ORDER BY provider, login",
        )?;
        let rows = st.query_map([], |r| {
            let scopes: String = r.get(5)?;
            Ok(Account {
                id: r.get(0)?,
                login: r.get(1)?,
                host: r.get(2)?,
                name: r.get(3)?,
                avatar_url: r.get(4)?,
                scopes: scopes.split(',').filter(|s| !s.is_empty()).map(Into::into).collect(),
                provider: Provider::parse(&r.get::<_, String>(6)?),
                token_source: TokenSource::parse(&r.get::<_, String>(7)?),
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn account(&self, id: &str) -> Result<Option<Account>> {
        Ok(self.accounts()?.into_iter().find(|a| a.id == id))
    }

    pub fn upsert_account(&self, a: &Account) -> Result<()> {
        self.conn().execute(
            "INSERT INTO accounts (id, login, host, name, avatar_url, scopes, provider, token_source)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(id) DO UPDATE SET name = ?4, avatar_url = ?5, scopes = ?6, provider = ?7, token_source = ?8",
            params![
                a.id,
                a.login,
                a.host,
                a.name,
                a.avatar_url,
                a.scopes.join(","),
                a.provider.as_str(),
                a.token_source.as_str()
            ],
        )?;
        Ok(())
    }

    /// Removes the account, its repos/projects (via cascade) and their cached items.
    pub fn delete_account(&self, id: &str) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        tx.execute(
            "DELETE FROM items WHERE source_id IN
               (SELECT id FROM repos WHERE account_id = ?1 UNION SELECT id FROM projects WHERE account_id = ?1)",
            [id],
        )?;
        tx.execute("DELETE FROM accounts WHERE id = ?1", [id])?;
        tx.commit()?;
        Ok(())
    }

    // ---- repos & projects -------------------------------------------------

    pub fn repos(&self) -> Result<Vec<Repo>> {
        let conn = self.conn();
        let mut st = conn.prepare_cached(
            "SELECT id, account_id, owner, name, url, default_category, provider FROM repos ORDER BY owner, name",
        )?;
        let rows = st.query_map([], |r| {
            Ok(Repo {
                id: r.get(0)?,
                account_id: r.get(1)?,
                owner: r.get(2)?,
                name: r.get(3)?,
                url: r.get(4)?,
                default_category: r.get(5)?,
                provider: Provider::parse(&r.get::<_, String>(6)?),
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn upsert_repo(&self, r: &Repo) -> Result<()> {
        self.conn().execute(
            "INSERT INTO repos (id, account_id, owner, name, url, default_category, provider) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(id) DO UPDATE SET account_id = ?2, owner = ?3, name = ?4, url = ?5, default_category = ?6, provider = ?7",
            params![r.id, r.account_id, r.owner, r.name, r.url, r.default_category, r.provider.as_str()],
        )?;
        Ok(())
    }

    pub fn projects(&self) -> Result<Vec<Project>> {
        let conn = self.conn();
        let mut st = conn.prepare_cached(
            "SELECT id, account_id, owner, number, title, url, default_category FROM projects ORDER BY title",
        )?;
        let rows = st.query_map([], |r| {
            Ok(Project {
                id: r.get(0)?,
                account_id: r.get(1)?,
                owner: r.get(2)?,
                number: r.get(3)?,
                title: r.get(4)?,
                url: r.get(5)?,
                default_category: r.get(6)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn upsert_project(&self, p: &Project) -> Result<()> {
        self.conn().execute(
            "INSERT INTO projects (id, account_id, owner, number, title, url, default_category)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(id) DO UPDATE SET account_id = ?2, owner = ?3, number = ?4, title = ?5, url = ?6,
               default_category = ?7",
            params![p.id, p.account_id, p.owner, p.number, p.title, p.url, p.default_category],
        )?;
        Ok(())
    }

    /// Deletes a repo or project and everything cached from it.
    pub fn delete_source(&self, id: &str) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM repos WHERE id = ?1", [id])?;
        tx.execute("DELETE FROM projects WHERE id = ?1", [id])?;
        tx.execute("DELETE FROM items WHERE source_id = ?1", [id])?;
        tx.execute("DELETE FROM sync_state WHERE source_id = ?1", [id])?;
        tx.commit()?;
        Ok(())
    }

    pub fn set_default_category(&self, container_id: &str, category: Option<&str>) -> Result<()> {
        let conn = self.conn();
        for table in ["repos", "projects", "reminder_lists", "event_calendars"] {
            conn.execute(
                &format!("UPDATE {table} SET default_category = ?2 WHERE id = ?1"),
                params![container_id, category],
            )?;
        }
        Ok(())
    }

    // ---- reminder lists ---------------------------------------------------

    pub fn reminder_lists(&self) -> Result<Vec<ReminderList>> {
        self.lists(ListKind::Reminders)
    }

    pub fn event_calendars(&self) -> Result<Vec<ReminderList>> {
        self.lists(ListKind::Events)
    }

    fn lists(&self, kind: ListKind) -> Result<Vec<ReminderList>> {
        let conn = self.conn();
        let mut st = conn.prepare_cached(&format!(
            "SELECT id, title, color, source_title, writable, enabled, default_category
             FROM {} WHERE present = 1 ORDER BY source_title, title",
            kind.table()
        ))?;
        let rows = st.query_map([], |r| {
            Ok(ReminderList {
                id: r.get(0)?,
                title: r.get(1)?,
                color: r.get(2)?,
                source_title: r.get(3)?,
                writable: r.get(4)?,
                enabled: r.get(5)?,
                default_category: r.get(6)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn sync_reminder_lists(&self, lists: &[ReminderList]) -> Result<()> {
        self.sync_lists(ListKind::Reminders, lists)
    }

    pub fn sync_event_calendars(&self, lists: &[ReminderList]) -> Result<()> {
        self.sync_lists(ListKind::Events, lists)
    }

    /// Records the lists EventKit currently reports, keeping user settings for
    /// lists we've seen before. Lists that disappeared are hidden, not deleted,
    /// so their category mapping survives a temporary iCloud hiccup.
    fn sync_lists(&self, kind: ListKind, lists: &[ReminderList]) -> Result<()> {
        let table = kind.table();
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        tx.execute(&format!("UPDATE {table} SET present = 0"), [])?;
        for l in lists {
            tx.execute(
                &format!(
                    "INSERT INTO {table} (id, title, color, source_title, writable, present)
                     VALUES (?1, ?2, ?3, ?4, ?5, 1)
                     ON CONFLICT(id) DO UPDATE SET title = ?2, color = ?3, source_title = ?4, writable = ?5, present = 1"
                ),
                params![l.id, l.title, l.color, l.source_title, l.writable],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Shows or hides a Reminders list or event calendar (ids don't overlap).
    pub fn set_list_enabled(&self, id: &str, enabled: bool) -> Result<()> {
        let conn = self.conn();
        for table in ["reminder_lists", "event_calendars"] {
            conn.execute(&format!("UPDATE {table} SET enabled = ?2 WHERE id = ?1"), params![id, enabled])?;
        }
        Ok(())
    }

    // ---- events -----------------------------------------------------------

    pub fn replace_events(&self, events: &[CalendarEvent]) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM events", [])?;
        {
            let mut st = tx.prepare_cached("INSERT OR REPLACE INTO events (id, data) VALUES (?1, ?2)")?;
            for e in events {
                st.execute(params![e.id, serde_json::to_string(e)?])?;
            }
        }
        tx.execute(
            "INSERT INTO sync_state (source_id, last_synced, error) VALUES ('calendar', ?1, NULL)
             ON CONFLICT(source_id) DO UPDATE SET last_synced = ?1, error = NULL",
            params![chrono::Utc::now().to_rfc3339()],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn put_event(&self, e: &CalendarEvent) -> Result<()> {
        self.conn().execute(
            "INSERT OR REPLACE INTO events (id, data) VALUES (?1, ?2)",
            params![e.id, serde_json::to_string(e)?],
        )?;
        Ok(())
    }

    pub fn remove_event(&self, id: &str) -> Result<()> {
        self.conn().execute("DELETE FROM events WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn event(&self, id: &str) -> Result<Option<CalendarEvent>> {
        let data: Option<String> = self
            .conn()
            .query_row("SELECT data FROM events WHERE id = ?1", [id], |r| r.get(0))
            .optional()?;
        Ok(data.map(|d| serde_json::from_str(&d)).transpose()?)
    }

    pub fn events(&self) -> Result<Vec<CalendarEvent>> {
        let conn = self.conn();
        let mut st = conn.prepare_cached("SELECT data FROM events")?;
        let rows = st.query_map([], |r| r.get::<_, String>(0))?;
        let mut out = Vec::new();
        for row in rows {
            if let Ok(e) = serde_json::from_str(&row?) {
                out.push(e);
            }
        }
        Ok(out)
    }

    // ---- items ------------------------------------------------------------

    /// Atomically replaces everything cached for one source.
    pub fn replace_items(&self, source_id: &str, items: &[Item]) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM items WHERE source_id = ?1", [source_id])?;
        {
            let mut st =
                tx.prepare_cached("INSERT OR REPLACE INTO items (source_id, id, data) VALUES (?1, ?2, ?3)")?;
            for it in items {
                st.execute(params![source_id, it.id, serde_json::to_string(it)?])?;
            }
        }
        tx.execute(
            "INSERT INTO sync_state (source_id, last_synced, error) VALUES (?1, ?2, NULL)
             ON CONFLICT(source_id) DO UPDATE SET last_synced = ?2, error = NULL",
            params![source_id, chrono::Utc::now().to_rfc3339()],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn put_item(&self, source_id: &str, item: &Item) -> Result<()> {
        self.conn().execute(
            "INSERT OR REPLACE INTO items (source_id, id, data) VALUES (?1, ?2, ?3)",
            params![source_id, item.id, serde_json::to_string(item)?],
        )?;
        Ok(())
    }

    pub fn remove_item(&self, id: &str) -> Result<()> {
        self.conn().execute("DELETE FROM items WHERE id = ?1", [id])?;
        Ok(())
    }

    /// Every cached copy of an item, keyed by the source that produced it.
    pub fn item_copies(&self, id: &str) -> Result<Vec<(String, Item)>> {
        let conn = self.conn();
        let mut st = conn.prepare_cached("SELECT source_id, data FROM items WHERE id = ?1")?;
        let rows = st.query_map([id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        let mut out = Vec::new();
        for row in rows {
            let (src, data) = row?;
            out.push((src, serde_json::from_str(&data)?));
        }
        Ok(out)
    }

    pub fn all_items(&self) -> Result<Vec<(String, Item)>> {
        let conn = self.conn();
        let mut st = conn.prepare_cached("SELECT source_id, data FROM items")?;
        let rows = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        let mut out = Vec::new();
        for row in rows {
            let (src, data) = row?;
            match serde_json::from_str(&data) {
                Ok(item) => out.push((src, item)),
                Err(e) => tracing::warn!("skipping unreadable cached item: {e}"),
            }
        }
        Ok(out)
    }

    // ---- sync state -------------------------------------------------------

    pub fn set_sync_error(&self, source_id: &str, error: &str) -> Result<()> {
        self.conn().execute(
            "INSERT INTO sync_state (source_id, error) VALUES (?1, ?2)
             ON CONFLICT(source_id) DO UPDATE SET error = ?2",
            params![source_id, error],
        )?;
        Ok(())
    }

    pub fn sync_state(&self, source_id: &str) -> Result<(Option<String>, Option<String>)> {
        Ok(self
            .conn()
            .query_row(
                "SELECT last_synced, error FROM sync_state WHERE source_id = ?1",
                [source_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?
            .unwrap_or((None, None)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn issue(id: &str, repo: &str) -> Item {
        Item {
            id: id.into(),
            kind: ItemKind::Issue,
            provider: Provider::Github,
            container_id: repo.into(),
            container_name: "o/r".into(),
            account_id: Some("me@github.com".into()),
            title: "t".into(),
            body: None,
            url: None,
            number: Some(1),
            completed: false,
            completed_at: None,
            due: None,
            priority: 0,
            labels: vec![],
            assignees: vec![],
            status: None,
            project_ids: vec![],
            updated_at: None,
            start: None,
            project_dates: None,
            parent_id: None,
            sub_issues: None,
            categories: vec![],
        }
    }

    #[test]
    fn seeds_default_categories_once() {
        let db = Db::in_memory().unwrap();
        assert_eq!(db.categories().unwrap().len(), 4);
        db.seed().unwrap();
        assert_eq!(db.categories().unwrap().len(), 4);
    }

    #[test]
    fn deleting_account_cascades_to_sources_and_items() {
        let db = Db::in_memory().unwrap();
        let acct = Account {
            id: "me@github.com".into(),
            login: "me".into(),
            host: "github.com".into(),
            name: None,
            avatar_url: None,
            scopes: vec!["repo".into()],
            provider: Provider::Github,
            token_source: TokenSource::Keychain,
        };
        db.upsert_account(&acct).unwrap();
        db.upsert_repo(&Repo {
            id: "R1".into(),
            account_id: acct.id.clone(),
            owner: "o".into(),
            name: "r".into(),
            url: "".into(),
            default_category: Some("school".into()),
            provider: Provider::Github,
        })
        .unwrap();
        db.replace_items("R1", &[issue("gh:1", "R1"), issue("gh:2", "R1")]).unwrap();
        db.put_item("reminders", &issue("rem:1", "L1")).unwrap();
        assert_eq!(db.all_items().unwrap().len(), 3);

        db.delete_account(&acct.id).unwrap();
        assert!(db.repos().unwrap().is_empty());
        let left = db.all_items().unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].1.id, "rem:1");
    }

    #[test]
    fn migrates_old_databases_to_have_providers() {
        let conn = Connection::open_in_memory().unwrap();
        // An accounts/repos table from before GitLab support.
        conn.execute_batch(
            "CREATE TABLE accounts (id TEXT PRIMARY KEY, login TEXT NOT NULL, host TEXT NOT NULL,
               name TEXT, avatar_url TEXT, scopes TEXT NOT NULL DEFAULT '');
             CREATE TABLE repos (id TEXT PRIMARY KEY, account_id TEXT NOT NULL, owner TEXT NOT NULL,
               name TEXT NOT NULL, url TEXT NOT NULL, default_category TEXT);
             INSERT INTO accounts (id, login, host) VALUES ('me@github.com', 'me', 'github.com');",
        )
        .unwrap();
        conn.execute_batch(SCHEMA).unwrap();
        migrate(&conn).unwrap();
        migrate(&conn).unwrap(); // idempotent
        let db = Db { conn: Mutex::new(conn) };
        let a = &db.accounts().unwrap()[0];
        assert_eq!(a.provider, Provider::Github);
        // An account linked before CLI imports keeps its Keychain token.
        assert_eq!(a.token_source, TokenSource::Keychain);
    }

    #[test]
    fn remembers_where_an_account_keeps_its_token() {
        let db = Db::in_memory().unwrap();
        let mut a = Account {
            id: "me@gitlab.com".into(),
            login: "me".into(),
            host: "gitlab.com".into(),
            name: None,
            avatar_url: None,
            scopes: vec![],
            provider: Provider::Gitlab,
            token_source: TokenSource::Cli,
        };
        db.upsert_account(&a).unwrap();
        assert_eq!(db.account(&a.id).unwrap().unwrap().token_source, TokenSource::Cli);
        // Re-linking by pasting a token moves it back to the Keychain.
        a.token_source = TokenSource::Keychain;
        db.upsert_account(&a).unwrap();
        assert_eq!(db.account(&a.id).unwrap().unwrap().token_source, TokenSource::Keychain);
    }

    #[test]
    fn deleting_category_clears_defaults() {
        let db = Db::in_memory().unwrap();
        db.sync_reminder_lists(&[ReminderList {
            id: "L1".into(),
            title: "Homework".into(),
            color: None,
            source_title: None,
            writable: true,
            enabled: true,
            default_category: None,
        }])
        .unwrap();
        db.set_default_category("L1", Some("school")).unwrap();
        assert_eq!(db.reminder_lists().unwrap()[0].default_category.as_deref(), Some("school"));
        db.delete_category("school").unwrap();
        assert_eq!(db.reminder_lists().unwrap()[0].default_category, None);
    }
}
