use crate::cmd::{icon, report};
use crate::{Ctx, db};
use anyhow::Result;

pub fn ls(ctx: &Ctx, topic: Option<&str>, all: bool, json: bool) -> Result<String> {
    let mut tasks: Vec<report::Item> = if all {
        db::tasks_where(&ctx.db, "1=1")?
            .into_iter()
            .map(|t| {
                let row = t.thread_id.as_deref().and_then(|id| db::thread(&ctx.db, id).ok().flatten());
                report::Item {
                    link: row.as_ref().and_then(|r| r.link.clone()),
                    thread_status: row.as_ref().map(|r| r.status.clone()),
                    thread_settled: row.as_ref().is_some_and(|r| r.settled),
                    group: if t.state == "open" { "active" } else { "closed" },
                    task: t,
                }
            })
            .collect()
    } else {
        report::items(ctx)?
    };
    if let Some(tp) = topic {
        tasks.retain(|i| i.task.topic.as_deref() == Some(tp));
    }
    if json {
        return Ok(serde_json::to_string_pretty(&tasks)? + "\n");
    }
    let mut s = String::new();
    for i in &tasks {
        let t = &i.task;
        let state = if t.state == "open" { String::new() } else { format!(" ({})", t.state) };
        let topic = t.topic.as_deref().map(|x| format!("[{x}] ")).unwrap_or_default();
        let status = i.thread_status.as_deref().map(|x| format!("  <{x}>")).unwrap_or_default();
        let next = t.next.as_deref().map(|n| format!(" → {n}")).unwrap_or_default();
        s += &format!("#{:<4} {} {topic}{}{next}{state}{status}\n", t.id, icon(&t.kind), t.title);
    }
    if s.is_empty() {
        s = "(no tasks)\n".into();
    }
    Ok(s)
}
