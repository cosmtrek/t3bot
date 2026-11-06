use crate::{Ctx, cmd::check_kind, db};
use anyhow::{Result, anyhow};
use rusqlite::params;

#[derive(Default)]
pub struct EditArgs {
    pub kind: Option<String>,
    pub topic: Option<String>,
    pub title: Option<String>,
    pub next: Option<String>,
    pub project: Option<String>,
}

/// Empty string clears topic/next/project.
pub fn edit(ctx: &Ctx, id: i64, a: EditArgs) -> Result<db::Task> {
    let mut t = db::task(&ctx.db, id)?.ok_or_else(|| anyhow!("no task #{id}"))?;
    if let Some(k) = a.kind {
        check_kind(&k)?;
        t.kind = k;
    }
    let clear = |v: String| if v.trim().is_empty() { None } else { Some(v) };
    if let Some(v) = a.topic {
        t.topic = clear(v);
    }
    if let Some(v) = a.title {
        anyhow::ensure!(!v.trim().is_empty(), "title can't be empty");
        t.title = v;
    }
    if let Some(v) = a.next {
        t.next = clear(v);
    }
    if let Some(v) = a.project {
        t.project_id = clear(v);
    }
    t.updated_at = ctx.now_ts();
    ctx.db.execute(
        "UPDATE tasks SET kind=?2, topic=?3, title=?4, next=?5, project_id=?6, updated_at=?7 WHERE id=?1",
        params![id, t.kind, t.topic, t.title, t.next, t.project_id, t.updated_at],
    )?;
    Ok(t)
}
