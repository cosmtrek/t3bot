//! Design §5 "同步规则". Pulls `t3 thread list` for every configured project into the
//! `threads` cache and applies the two automatic task transitions (gone → dropped,
//! settled → closeable done).

use crate::Ctx;
use anyhow::Result;
use rusqlite::params;
use std::collections::HashSet;

#[derive(Debug, Default)]
pub struct SyncReport {
    /// (project path, error) for projects whose list failed; their threads are left untouched.
    pub failed: Vec<(String, String)>,
    pub seen: usize,
    pub gone: usize,
    pub dropped: usize,
    pub auto_closed: usize,
}

pub fn sync(ctx: &Ctx) -> Result<SyncReport> {
    let mut rep = SyncReport::default();
    let now = ctx.now_ts();
    let tx = ctx.db.unchecked_transaction()?;
    let mut seen: HashSet<String> = HashSet::new();

    for path in &ctx.cfg.projects {
        let list = match ctx.t3.list(path) {
            Ok(l) => l,
            Err(e) => {
                rep.failed.push((path.clone(), format!("{e:#}")));
                continue;
            }
        };
        for t in list {
            tx.execute(
                "INSERT INTO threads (thread_id, project_id, project_path, title, link, status, settled, snoozed, fingerprint, gone_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, NULL)
                 ON CONFLICT(thread_id) DO UPDATE SET
                   project_id = excluded.project_id, project_path = excluded.project_path,
                   title = excluded.title, link = excluded.link, status = excluded.status,
                   settled = excluded.settled, snoozed = excluded.snoozed,
                   fingerprint = excluded.fingerprint, gone_at = NULL",
                params![
                    t.thread_id, t.project_id, path, t.title, t.link, t.status,
                    t.settled, t.snoozed, t.fingerprint()
                ],
            )?;
            seen.insert(t.thread_id);
        }
    }
    rep.seen = seen.len();

    // Gone: not seen in a project we listed successfully, or its project left the config.
    // Threads of a project whose list failed are NOT marked gone.
    let failed: HashSet<&str> = rep.failed.iter().map(|(p, _)| p.as_str()).collect();
    let mut gone_ids = Vec::new();
    {
        let mut st = tx.prepare("SELECT thread_id, project_path FROM threads WHERE gone_at IS NULL")?;
        let rows = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        for row in rows {
            let (id, path) = row?;
            if failed.contains(path.as_str()) || seen.contains(&id) {
                continue;
            }
            gone_ids.push(id);
        }
    }
    for id in &gone_ids {
        tx.execute("UPDATE threads SET gone_at = ?2 WHERE thread_id = ?1", params![id, now])?;
        rep.dropped += tx.execute(
            "UPDATE tasks SET state = 'dropped', close_note = 'thread_gone', closed_at = ?2, updated_at = ?2
             WHERE thread_id = ?1 AND state = 'open'",
            params![id, now],
        )?;
    }
    rep.gone = gone_ids.len();

    // Settled closes only `closeable` tasks; todo/discuss stay open (T3 auto-settles).
    rep.auto_closed = tx.execute(
        "UPDATE tasks SET state = 'done', close_note = 'settled', closed_at = ?1, updated_at = ?1
         WHERE state = 'open' AND kind = 'closeable'
           AND thread_id IN (SELECT thread_id FROM threads WHERE settled = 1 AND gone_at IS NULL)",
        params![now],
    )?;
    tx.commit()?;
    Ok(rep)
}
