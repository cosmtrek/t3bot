use crate::t3::StartArgs;
use crate::{Ctx, db};
use anyhow::{Result, anyhow, bail};
use rusqlite::params;

#[derive(Debug, PartialEq)]
pub enum Opened {
    Sent { id: i64, thread_id: String },
    Started { id: i64, thread_id: String },
}

fn join(parts: &[Option<&str>]) -> String {
    parts
        .iter()
        .flatten()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// Design §6 `open`: send `next` (+ say) to the bound thread, or start a thread for a
/// note. Order is send-then-write, so a failed send leaves the task untouched.
pub fn open(ctx: &Ctx, ids: &[i64], say: Option<&str>) -> Result<Vec<Opened>> {
    let mut out = Vec::new();
    for &id in ids {
        let t = db::task(&ctx.db, id)?.ok_or_else(|| anyhow!("no task #{id}"))?;
        if t.state != "open" {
            bail!("task #{id} is {}, not open", t.state);
        }
        let now = ctx.now_ts();
        match &t.thread_id {
            Some(tid) => {
                let msg = join(&[t.next.as_deref(), say]);
                if msg.is_empty() {
                    bail!("task #{id} has no `next`; pass --say TEXT");
                }
                ctx.t3.send(tid, &msg)?;
                ctx.db.execute("UPDATE tasks SET next=NULL, updated_at=?2 WHERE id=?1", params![id, now])?;
                out.push(Opened::Sent { id, thread_id: tid.clone() });
            }
            None => {
                let project = t
                    .project_id
                    .as_deref()
                    .ok_or_else(|| anyhow!("task #{id} has no thread and no project; `t3bot edit {id} --project <path>` first"))?;
                let msg = join(&[Some(t.title.as_str()), t.next.as_deref(), say]);
                let tid = ctx.t3.start(&StartArgs {
                    project,
                    message: &msg,
                    title: Some(&t.title),
                    provider: None,
                    model: None,
                })?;
                ctx.db.execute(
                    "UPDATE tasks SET thread_id=?2, next=NULL, updated_at=?3 WHERE id=?1",
                    params![id, tid, now],
                )?;
                out.push(Opened::Started { id, thread_id: tid });
            }
        }
    }
    Ok(out)
}
