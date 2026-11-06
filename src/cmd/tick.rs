//! Design §7 flow A. cron entry point: sync, and only if something changed (or
//! --always) wake today's inbox thread. Never waits for the agent (D2).

use crate::t3::StartArgs;
use crate::{Ctx, db, sync};
use anyhow::{Context, Result, anyhow};

pub const DEFAULT_PROMPT: &str = include_str!("../../prompts/wake.md");

#[derive(Debug, PartialEq)]
pub enum Tick {
    NoChange,
    Woke { inbox: String, created: bool, pending: usize },
}

pub fn prompt_body() -> String {
    let p = crate::config::home().join("prompt.md");
    std::fs::read_to_string(&p).unwrap_or_else(|_| DEFAULT_PROMPT.to_string())
}

pub fn tick(ctx: &Ctx, always: bool, body: &str) -> Result<Tick> {
    let rep = sync::sync(ctx)?;
    let pending = db::pending_threads(&ctx.db)?.len();
    if pending == 0 && !always {
        return Ok(Tick::NoChange);
    }
    let date = ctx.now.strftime("%Y-%m-%d").to_string();
    let existing = db::inbox_for(&ctx.db, &date)?;
    let created = existing.is_none();
    let yesterday = db::previous_inbox(&ctx.db, &date)?;

    let mut header = format!("t3bot 巡检 · {} · 待分类 {pending}", ctx.now.strftime("%Y-%m-%d %H:%M"));
    if created {
        header += " · 当天第一轮";
        match &yesterday {
            Some((d, id)) => header += &format!(" · 昨天收件箱（{d}）：{id}"),
            None => header += " · 没有昨天的收件箱",
        }
    }
    for (p, _) in &rep.failed {
        header += &format!(" · ⚠️ 项目列表失败 {p}");
    }
    let msg = format!("{header}\n\n{body}");

    let inbox = match existing {
        Some(id) => {
            ctx.t3.send(&id, &msg).with_context(|| format!("waking inbox {id}"))?;
            id
        }
        None => {
            let project = ctx
                .cfg
                .inbox_project
                .as_deref()
                .ok_or_else(|| anyhow!("config: inbox_project is not set → run `t3bot init`"))?;
            let title = format!("📋 {}", ctx.now.strftime("%m/%d"));
            let id = ctx.t3.start(&StartArgs {
                project,
                message: &msg,
                title: Some(&title),
                provider: ctx.cfg.provider.as_deref(),
                model: ctx.cfg.model.as_deref(),
            })?;
            db::insert_inbox(&ctx.db, &date, &id, &ctx.now_ts())?;
            id
        }
    };
    Ok(Tick::Woke { inbox, created, pending })
}
