//! Design §6 `annotate`: write classifier output, guarded only by the fingerprint.
//! Only touches kind/topic/title/next of OPEN tasks; never changes state.

use crate::{Ctx, cmd::check_kind, cmd::none_if_empty, db};
use anyhow::Result;
use rusqlite::params;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
pub struct Annotation {
    pub thread_id: String,
    pub fingerprint: String,
    pub kind: String,
    #[serde(default)]
    pub topic: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub next: Option<String>,
}

#[derive(Debug, Default, Serialize)]
pub struct AnnotateResult {
    pub created: Vec<i64>,
    pub updated: Vec<i64>,
    /// Fingerprint no longer matches: dropped, will come back in the next pending.
    pub stale: Vec<String>,
    pub errors: Vec<String>,
}

pub fn parse(input: &str) -> Result<Vec<Annotation>> {
    let v: serde_json::Value = serde_json::from_str(input.trim())?;
    // Accept a bare array, a single object, or {"threads": [...]}.
    let v = match v {
        serde_json::Value::Object(mut m) if m.contains_key("threads") => m.remove("threads").unwrap(),
        serde_json::Value::Object(m) => serde_json::Value::Array(vec![serde_json::Value::Object(m)]),
        other => other,
    };
    Ok(serde_json::from_value(v)?)
}

pub fn annotate(ctx: &Ctx, items: Vec<Annotation>) -> Result<AnnotateResult> {
    let mut res = AnnotateResult::default();
    let now = ctx.now_ts();
    for a in items {
        if let Err(e) = check_kind(&a.kind) {
            res.errors.push(format!("{}: {e}", a.thread_id));
            continue;
        }
        let Some(row) = db::thread(&ctx.db, &a.thread_id)? else {
            res.errors.push(format!("{}: unknown thread (not in cache)", a.thread_id));
            continue;
        };
        if row.fingerprint != a.fingerprint {
            res.stale.push(a.thread_id);
            continue;
        }
        let topic = none_if_empty(a.topic);
        let next = none_if_empty(a.next);
        let title = none_if_empty(a.title);
        let tx = ctx.db.unchecked_transaction()?;
        match db::open_task_for_thread(&tx, &a.thread_id)? {
            Some(t) => {
                tx.execute(
                    "UPDATE tasks SET kind=?2, topic=COALESCE(?3, topic), title=COALESCE(?4, title), next=?5, updated_at=?6
                     WHERE id=?1 AND state='open'",
                    params![t.id, a.kind, topic, title, next, now],
                )?;
                res.updated.push(t.id);
            }
            None => {
                tx.execute(
                    "INSERT INTO tasks (thread_id, project_id, kind, topic, title, next, source, opened_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'sweep', ?7, ?7)",
                    params![a.thread_id, row.project_id, a.kind, topic, title.unwrap_or(row.title), next, now],
                )?;
                res.created.push(tx.last_insert_rowid());
            }
        }
        tx.execute(
            "UPDATE threads SET analyzed_fingerprint = fingerprint WHERE thread_id = ?1",
            [&a.thread_id],
        )?;
        tx.commit()?;
    }
    Ok(res)
}
