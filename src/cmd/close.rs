use crate::{Ctx, db};
use anyhow::{Result, anyhow};
use rusqlite::params;
use serde::Serialize;

#[derive(Debug, Serialize, PartialEq)]
pub struct Settle {
    pub thread_id: String,
    pub title: String,
}

#[derive(Debug, Serialize)]
pub struct CloseResult {
    pub closed: Vec<i64>,
    pub state: &'static str,
    /// Already closed or unknown ids, with a reason.
    pub skipped: Vec<String>,
    /// The agent settles these with MCP `t3_thread_organize(action:"settle")` (design §4).
    pub threads_to_settle: Vec<Settle>,
}

pub fn close(ctx: &Ctx, ids: &[i64], drop: bool) -> Result<CloseResult> {
    let state = if drop { "dropped" } else { "done" };
    let now = ctx.now_ts();
    let mut res = CloseResult { closed: vec![], state, skipped: vec![], threads_to_settle: vec![] };
    for &id in ids {
        let t = match db::task(&ctx.db, id)? {
            Some(t) => t,
            None => {
                res.skipped.push(format!("#{id}: no such task"));
                continue;
            }
        };
        if t.state != "open" {
            res.skipped.push(format!("#{id}: already {}", t.state));
            continue;
        }
        ctx.db.execute(
            "UPDATE tasks SET state=?2, close_note=?3, closed_at=?4, updated_at=?4 WHERE id=?1",
            params![id, state, if drop { Some("manual") } else { None }, now],
        )?;
        res.closed.push(id);
        if let Some(tid) = &t.thread_id {
            let row = db::thread(&ctx.db, tid)?;
            let needs = match &row {
                Some(r) => !r.settled && r.gone_at.is_none(),
                None => true,
            };
            if needs && !res.threads_to_settle.iter().any(|s| &s.thread_id == tid) {
                let title = row.map(|r| r.title).unwrap_or_else(|| t.title.clone());
                res.threads_to_settle.push(Settle { thread_id: tid.clone(), title });
            }
        }
    }
    Ok(res)
}

impl CloseResult {
    pub fn text(&self) -> String {
        let mut s = String::new();
        if !self.closed.is_empty() {
            let ids: Vec<String> = self.closed.iter().map(|i| format!("#{i}")).collect();
            s += &format!("{} → {}\n", ids.join(" "), self.state);
        }
        for k in &self.skipped {
            s += &format!("skipped {k}\n");
        }
        if !self.threads_to_settle.is_empty() {
            s += "threads_to_settle (use MCP t3_thread_organize action=settle):\n";
            for t in &self.threads_to_settle {
                s += &format!("  {}  {}\n", t.thread_id, t.title);
            }
        }
        s
    }
}

pub fn ensure_found(r: &CloseResult) -> Result<()> {
    if r.closed.is_empty() {
        return Err(anyhow!("nothing closed: {}", r.skipped.join("; ")));
    }
    Ok(())
}
