use crate::{Ctx, cmd::check_kind, cmd::none_if_empty, db};
use anyhow::{Result, bail};
use rusqlite::params;

pub struct AddArgs {
    pub title: String,
    pub kind: String,
    pub topic: Option<String>,
    pub project: Option<String>,
    pub thread: Option<String>,
    pub next: Option<String>,
}

pub fn add(ctx: &Ctx, a: AddArgs) -> Result<i64> {
    check_kind(&a.kind)?;
    let mut project = none_if_empty(a.project);
    if let Some(tid) = &a.thread {
        if let Some(t) = db::open_task_for_thread(&ctx.db, tid)? {
            bail!("thread {tid} already has open task #{}; use `t3bot edit {}`", t.id, t.id);
        }
        if let Some(row) = db::thread(&ctx.db, tid)? {
            project = Some(row.project_id); // inherit from the thread
        }
    }
    let now = ctx.now_ts();
    ctx.db.execute(
        "INSERT INTO tasks (thread_id, project_id, kind, topic, title, next, source, opened_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'manual', ?7, ?7)",
        params![a.thread, project, a.kind, none_if_empty(a.topic), a.title, none_if_empty(a.next), now],
    )?;
    Ok(ctx.db.last_insert_rowid())
}
