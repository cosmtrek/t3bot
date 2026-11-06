//! Design §6 `pending --json`: what the inbox agent needs to classify.

use crate::cmd::project_label;
use crate::{Ctx, db};
use anyhow::Result;
use serde::Serialize;

#[derive(Serialize)]
pub struct OpenTask {
    pub id: i64,
    pub kind: String,
    pub topic: Option<String>,
    pub title: String,
    pub next: Option<String>,
}

#[derive(Serialize)]
pub struct Msg {
    pub role: String,
    pub text: String,
}

#[derive(Serialize)]
pub struct PendingThread {
    pub thread_id: String,
    pub fingerprint: String,
    pub title: String,
    pub status: String,
    pub project: String,
    pub open_task: Option<OpenTask>,
    pub messages: Vec<Msg>,
}

#[derive(Serialize)]
pub struct Pending {
    pub topics: Vec<String>,
    pub threads: Vec<PendingThread>,
}

pub const MESSAGES: u32 = 4;
pub const MAX_CHARS: usize = 2000;

pub fn truncate(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((i, _)) => format!("{}…", &s[..i]),
        None => s.to_string(),
    }
}

pub fn pending(ctx: &Ctx, limit: Option<usize>) -> Result<Pending> {
    let mut rows = db::pending_threads(&ctx.db)?;
    if let Some(n) = limit {
        rows.truncate(n);
    }
    let mut threads = Vec::new();
    for r in rows {
        let messages = match ctx.t3.show(&r.thread_id, MESSAGES) {
            Ok(show) => show
                .messages
                .into_iter()
                .map(|m| Msg { role: m.role, text: truncate(&m.text, MAX_CHARS) })
                .collect(),
            Err(e) => vec![Msg { role: "error".into(), text: format!("t3 thread show failed: {e:#}") }],
        };
        let open_task = db::open_task_for_thread(&ctx.db, &r.thread_id)?.map(|t| OpenTask {
            id: t.id,
            kind: t.kind,
            topic: t.topic,
            title: t.title,
            next: t.next,
        });
        threads.push(PendingThread {
            thread_id: r.thread_id,
            fingerprint: r.fingerprint,
            title: r.title,
            status: r.status,
            project: project_label(&r.project_path),
            open_task,
            messages,
        });
    }
    Ok(Pending { topics: db::open_topics(&ctx.db)?, threads })
}
