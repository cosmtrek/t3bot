//! Schema (design §5) and the queries shared by several commands.

use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::Serialize;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS tasks (
  id          INTEGER PRIMARY KEY,
  thread_id   TEXT,
  project_id  TEXT,
  kind        TEXT NOT NULL,
  state       TEXT NOT NULL DEFAULT 'open',
  topic       TEXT,
  title       TEXT NOT NULL,
  next        TEXT,
  source      TEXT NOT NULL,
  close_note  TEXT,
  opened_at   TEXT NOT NULL,
  updated_at  TEXT NOT NULL,
  closed_at   TEXT
);
CREATE UNIQUE INDEX IF NOT EXISTS one_open_task_per_thread
  ON tasks(thread_id) WHERE state = 'open' AND thread_id IS NOT NULL;

CREATE TABLE IF NOT EXISTS threads (
  thread_id            TEXT PRIMARY KEY,
  project_id           TEXT NOT NULL,
  project_path         TEXT NOT NULL,
  title                TEXT NOT NULL,
  link                 TEXT,
  status               TEXT NOT NULL,
  settled              INTEGER NOT NULL,
  snoozed              INTEGER NOT NULL,
  fingerprint          TEXT NOT NULL,
  analyzed_fingerprint TEXT,
  gone_at              TEXT
);

CREATE TABLE IF NOT EXISTS inboxes (
  date       TEXT PRIMARY KEY,
  thread_id  TEXT NOT NULL,
  created_at TEXT NOT NULL
);
"#;

pub const KINDS: [&str; 3] = ["todo", "discuss", "closeable"];
pub const BUSY: [&str; 4] = ["preparing", "queued", "starting", "running"];

pub fn open(path: &std::path::Path) -> Result<Connection> {
    let db = Connection::open(path)?;
    db.pragma_update(None, "journal_mode", "WAL")?;
    db.busy_timeout(std::time::Duration::from_secs(5))?;
    migrate(&db)?;
    Ok(db)
}

pub fn open_memory() -> Result<Connection> {
    let db = Connection::open_in_memory()?;
    migrate(&db)?;
    Ok(db)
}

fn migrate(db: &Connection) -> Result<()> {
    db.execute_batch(SCHEMA)?;
    Ok(())
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Task {
    pub id: i64,
    pub thread_id: Option<String>,
    pub project_id: Option<String>,
    pub kind: String,
    pub state: String,
    pub topic: Option<String>,
    pub title: String,
    pub next: Option<String>,
    pub source: String,
    pub close_note: Option<String>,
    pub opened_at: String,
    pub updated_at: String,
    pub closed_at: Option<String>,
}

pub const TASK_COLS: &str = "id, thread_id, project_id, kind, state, topic, title, next, source, close_note, opened_at, updated_at, closed_at";

impl Task {
    pub fn from_row(r: &Row) -> rusqlite::Result<Task> {
        Ok(Task {
            id: r.get(0)?,
            thread_id: r.get(1)?,
            project_id: r.get(2)?,
            kind: r.get(3)?,
            state: r.get(4)?,
            topic: r.get(5)?,
            title: r.get(6)?,
            next: r.get(7)?,
            source: r.get(8)?,
            close_note: r.get(9)?,
            opened_at: r.get(10)?,
            updated_at: r.get(11)?,
            closed_at: r.get(12)?,
        })
    }
}

pub fn task(db: &Connection, id: i64) -> Result<Option<Task>> {
    Ok(db
        .query_row(&format!("SELECT {TASK_COLS} FROM tasks WHERE id = ?1"), [id], Task::from_row)
        .optional()?)
}

pub fn open_task_for_thread(db: &Connection, thread_id: &str) -> Result<Option<Task>> {
    Ok(db
        .query_row(
            &format!("SELECT {TASK_COLS} FROM tasks WHERE thread_id = ?1 AND state = 'open'"),
            [thread_id],
            Task::from_row,
        )
        .optional()?)
}

pub fn tasks_where(db: &Connection, cond: &str) -> Result<Vec<Task>> {
    let mut st = db.prepare(&format!("SELECT {TASK_COLS} FROM tasks WHERE {cond} ORDER BY id"))?;
    let rows = st.query_map([], Task::from_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

pub fn open_topics(db: &Connection) -> Result<Vec<String>> {
    let mut st = db.prepare(
        "SELECT DISTINCT topic FROM tasks WHERE state = 'open' AND topic IS NOT NULL AND topic != '' ORDER BY topic",
    )?;
    let rows = st.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<Vec<String>>>()?;
    Ok(rows)
}

#[derive(Debug, Clone)]
pub struct ThreadRow {
    pub thread_id: String,
    pub project_id: String,
    pub project_path: String,
    pub title: String,
    pub link: Option<String>,
    pub status: String,
    pub settled: bool,
    pub snoozed: bool,
    pub fingerprint: String,
    pub analyzed_fingerprint: Option<String>,
    pub gone_at: Option<String>,
}

const THREAD_COLS: &str = "thread_id, project_id, project_path, title, link, status, settled, snoozed, fingerprint, analyzed_fingerprint, gone_at";

impl ThreadRow {
    fn from_row(r: &Row) -> rusqlite::Result<ThreadRow> {
        Ok(ThreadRow {
            thread_id: r.get(0)?,
            project_id: r.get(1)?,
            project_path: r.get(2)?,
            title: r.get(3)?,
            link: r.get(4)?,
            status: r.get(5)?,
            settled: r.get(6)?,
            snoozed: r.get(7)?,
            fingerprint: r.get(8)?,
            analyzed_fingerprint: r.get(9)?,
            gone_at: r.get(10)?,
        })
    }

    pub fn busy(&self) -> bool {
        BUSY.contains(&self.status.as_str())
    }
}

pub fn thread(db: &Connection, id: &str) -> Result<Option<ThreadRow>> {
    Ok(db
        .query_row(&format!("SELECT {THREAD_COLS} FROM threads WHERE thread_id = ?1"), [id], ThreadRow::from_row)
        .optional()?)
}

/// Design §5 "待分类条件".
pub fn pending_threads(db: &Connection) -> Result<Vec<ThreadRow>> {
    let busy = BUSY.map(|s| format!("'{s}'")).join(",");
    let sql = format!(
        "SELECT {THREAD_COLS} FROM threads
         WHERE gone_at IS NULL
           AND fingerprint IS NOT analyzed_fingerprint
           AND status NOT IN ({busy})
           AND settled = 0 AND snoozed = 0
           AND thread_id NOT IN (SELECT thread_id FROM inboxes)
         ORDER BY project_path, thread_id"
    );
    let mut st = db.prepare(&sql)?;
    let rows = st.query_map([], ThreadRow::from_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

pub fn inbox_for(db: &Connection, date: &str) -> Result<Option<String>> {
    Ok(db
        .query_row("SELECT thread_id FROM inboxes WHERE date = ?1", [date], |r| r.get(0))
        .optional()?)
}

pub fn previous_inbox(db: &Connection, before: &str) -> Result<Option<(String, String)>> {
    Ok(db
        .query_row(
            "SELECT date, thread_id FROM inboxes WHERE date < ?1 ORDER BY date DESC LIMIT 1",
            [before],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?)
}

pub fn insert_inbox(db: &Connection, date: &str, thread_id: &str, now: &str) -> Result<()> {
    db.execute("INSERT INTO inboxes (date, thread_id, created_at) VALUES (?1, ?2, ?3)", params![date, thread_id, now])?;
    Ok(())
}
