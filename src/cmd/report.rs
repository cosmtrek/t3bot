//! Design §8. The report body is fixed in code so it stays stable between runs.

use crate::cmd::icon;
use crate::sync::SyncReport;
use crate::{Ctx, db};
use anyhow::Result;
use jiff::Timestamp;
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Debug, Serialize)]
pub struct Item {
    #[serde(flatten)]
    pub task: db::Task,
    pub link: Option<String>,
    pub thread_status: Option<String>,
    pub thread_settled: bool,
    pub group: &'static str, // active | running | closeable
}

pub fn items(ctx: &Ctx) -> Result<Vec<Item>> {
    let mut out = Vec::new();
    for t in db::tasks_where(&ctx.db, "state = 'open'")? {
        let row = match &t.thread_id {
            Some(tid) => db::thread(&ctx.db, tid)?,
            None => None,
        };
        let busy = row.as_ref().is_some_and(|r| r.busy());
        let group = if busy {
            "running"
        } else if t.kind == "closeable" {
            "closeable"
        } else {
            "active"
        };
        out.push(Item {
            link: row.as_ref().and_then(|r| r.link.clone()),
            thread_status: row.as_ref().map(|r| r.status.clone()),
            thread_settled: row.as_ref().is_some_and(|r| r.settled),
            group,
            task: t,
        });
    }
    Ok(out)
}

fn age(from: &str, now: &Timestamp) -> String {
    let Ok(t) = from.parse::<Timestamp>() else { return "?".into() };
    let secs = now.as_second() - t.as_second();
    if secs < 3600 {
        "刚刚".into()
    } else if secs < 48 * 3600 {
        format!("{}h", secs / 3600)
    } else {
        format!("{} 天", secs / 86400)
    }
}

fn num(t: &db::Task, link: &Option<String>) -> String {
    match link {
        Some(l) => format!("[#{}]({l})", t.id),
        None => format!("#{}", t.id),
    }
}

pub fn render(ctx: &Ctx, sync: &SyncReport) -> Result<String> {
    let items = items(ctx)?;
    let now = ctx.now.timestamp();
    let count = |k: &str| items.iter().filter(|i| i.group == "active" && i.task.kind == k).count();
    let closeable: Vec<&Item> = items.iter().filter(|i| i.group == "closeable").collect();
    let running: Vec<&Item> = items.iter().filter(|i| i.group == "running").collect();

    let mut s = format!(
        "📋 {}   待办 {} · 待议 {} · 可关 {}\n",
        ctx.now.strftime("%m/%d %H:%M"),
        count("todo"),
        count("discuss"),
        closeable.len()
    );

    // Active items grouped by topic; untopiced last.
    let mut groups: BTreeMap<(bool, String), Vec<&Item>> = BTreeMap::new();
    for i in items.iter().filter(|i| i.group == "active") {
        let topic = i.task.topic.clone().unwrap_or_default();
        groups.entry((topic.is_empty(), topic)).or_default().push(i);
    }
    for ((_, topic), list) in &groups {
        s += &format!("\n[{}]\n", if topic.is_empty() { "其他" } else { topic });
        for i in list {
            let t = &i.task;
            let settled = if i.thread_settled { "📦 已收起未闭环：" } else { "" };
            let note = if t.thread_id.is_none() { "📝 " } else { "" };
            let waiting = if i.thread_status.as_deref() == Some("waiting") { "⏳ " } else { "" };
            let a = age(&t.opened_at, &now);
            let when = match (t.kind.as_str(), a.as_str()) {
                (_, "刚刚") => "刚刚".to_string(),
                ("discuss", _) => format!("已议 {a}"),
                _ => format!("停 {a}"),
            };
            let next = t.next.as_deref().map(|n| format!(" → {n}")).unwrap_or_default();
            s += &format!(
                "- {} {} {waiting}{note}{settled}{}（{when}）{next}\n",
                num(t, &i.link),
                icon(&t.kind),
                t.title
            );
        }
    }

    if !running.is_empty() {
        let ids: Vec<String> = running.iter().map(|i| num(&i.task, &i.link)).collect();
        s += &format!("\n进行中：{}\n", ids.join(" "));
    }
    if !closeable.is_empty() {
        s += "\n可关：\n";
        for i in &closeable {
            s += &format!("- {} {}\n", num(&i.task, &i.link), i.task.title);
        }
    }
    if items.is_empty() {
        s += "\n没有未闭环的事项。\n";
    }
    let pending = db::pending_threads(&ctx.db)?.len();
    if pending > 0 {
        s += &format!("\n待分类：{pending} 个 thread（下一轮巡检处理）\n");
    }
    for (path, err) in &sync.failed {
        s += &format!("\n⚠️ 项目列表失败：{path}（{}）\n", first_line(err));
    }
    Ok(s)
}

fn first_line(s: &str) -> &str {
    s.lines().next().unwrap_or("")
}
