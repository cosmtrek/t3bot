use t3bot::cmd::{add, annotate, close, edit, open, pending, report, tick};
use t3bot::t3::{FakeT3, Message, ThreadInfo};
use std::rc::Rc;
use t3bot::{Ctx, config::Config, db, sync::sync};

const P1: &str = "/repo/a";
const P2: &str = "/repo/b";

fn th(id: &str, project: &str, status: &str, run: &str, items: u64) -> ThreadInfo {
    ThreadInfo {
        thread_id: id.into(),
        project_id: project.into(),
        title: format!("title {id}"),
        link: Some(format!("https://x/{id}")),
        status: status.into(),
        latest_run_id: Some(run.into()),
        settled: false,
        snoozed: false,
        item_count: items,
    }
}

fn ctx(fake: FakeT3) -> (Ctx, Rc<FakeT3>) {
    let fake = Rc::new(fake);
    let c = Ctx {
        db: db::open_memory().unwrap(),
        t3: Box::new(fake.clone()),
        cfg: Config {
            inbox_project: Some("inbox-proj".into()),
            provider: Some("claudeAgent".into()),
            model: Some("haiku".into()),
            projects: vec![P1.into(), P2.into()],
            t3_bin: None,
        },
        now: "2026-11-06T09:00:00+08:00[Asia/Singapore]".parse().unwrap(),
    };
    (c, fake)
}

fn set_list(c: &FakeT3, path: &str, v: Result<Vec<ThreadInfo>, String>) {
    c.lists.borrow_mut().insert(path.into(), v);
}

fn task_for(c: &Ctx, tid: &str, kind: &str) -> i64 {
    let id = add::add(c, add::AddArgs {
        title: format!("task {tid}"),
        kind: kind.into(),
        topic: None,
        project: None,
        thread: Some(tid.into()),
        next: Some("do it".into()),
    })
    .unwrap();
    id
}

fn state(c: &Ctx, id: i64) -> (String, Option<String>) {
    let t = db::task(&c.db, id).unwrap().unwrap();
    (t.state, t.close_note)
}

#[test]
fn settled_only_closes_closeable() {
    let (c, f) = ctx(FakeT3::default());
    set_list(&f, P1, Ok(vec![th("t1", "pa", "completed", "r1", 3), th("t2", "pa", "completed", "r1", 3), th("t3", "pa", "completed", "r1", 3)]));
    set_list(&f, P2, Ok(vec![]));
    sync(&c).unwrap();
    let todo = task_for(&c, "t1", "todo");
    let disc = task_for(&c, "t2", "discuss");
    let cl = task_for(&c, "t3", "closeable");

    let mut all = vec![th("t1", "pa", "completed", "r1", 3), th("t2", "pa", "completed", "r1", 3), th("t3", "pa", "completed", "r1", 3)];
    for t in &mut all {
        t.settled = true;
    }
    set_list(&f, P1, Ok(all));
    let r = sync(&c).unwrap();
    assert_eq!(r.auto_closed, 1);
    assert_eq!(state(&c, todo).0, "open");
    assert_eq!(state(&c, disc).0, "open");
    assert_eq!(state(&c, cl), ("done".into(), Some("settled".into())));
    // Report marks the settled-but-open todo.
    let out = report::render(&c, &r).unwrap();
    assert!(out.contains("📦 已收起未闭环"), "{out}");
}

#[test]
fn gone_drops_tasks_but_failed_project_does_not() {
    let (c, f) = ctx(FakeT3::default());
    set_list(&f, P1, Ok(vec![th("a1", "pa", "completed", "r", 1), th("a2", "pa", "completed", "r", 1)]));
    set_list(&f, P2, Ok(vec![th("b1", "pb", "completed", "r", 1)]));
    sync(&c).unwrap();
    let a1 = task_for(&c, "a1", "todo");
    let a2 = task_for(&c, "a2", "todo");
    let b1 = task_for(&c, "b1", "todo");

    // P2's list fails, P1 lost a2.
    set_list(&f, P1, Ok(vec![th("a1", "pa", "completed", "r", 1)]));
    set_list(&f, P2, Err("server down".into()));
    let r = sync(&c).unwrap();
    assert_eq!(r.failed.len(), 1);
    assert_eq!(r.gone, 1);
    assert_eq!(state(&c, a1).0, "open");
    assert_eq!(state(&c, a2), ("dropped".into(), Some("thread_gone".into())));
    assert_eq!(state(&c, b1).0, "open", "failed project must not mark threads gone");
    assert!(db::thread(&c.db, "b1").unwrap().unwrap().gone_at.is_none());
    assert!(report::render(&c, &r).unwrap().contains("⚠️ 项目列表失败：/repo/b"));

    // Project emptied out entirely (list OK, zero threads) → gone.
    set_list(&f, P2, Ok(vec![]));
    sync(&c).unwrap();
    assert_eq!(state(&c, b1).0, "dropped");
}

#[test]
fn reappearing_thread_clears_gone() {
    let (c, f) = ctx(FakeT3::default());
    set_list(&f, P1, Ok(vec![th("a1", "pa", "completed", "r", 1)]));
    set_list(&f, P2, Ok(vec![]));
    sync(&c).unwrap();
    set_list(&f, P1, Ok(vec![]));
    sync(&c).unwrap();
    assert!(db::thread(&c.db, "a1").unwrap().unwrap().gone_at.is_some());
    set_list(&f, P1, Ok(vec![th("a1", "pa", "completed", "r", 1)]));
    sync(&c).unwrap();
    assert!(db::thread(&c.db, "a1").unwrap().unwrap().gone_at.is_none());
}

#[test]
fn pending_filter() {
    let (c, f) = ctx(FakeT3::default());
    let mut settled = th("settled", "pa", "completed", "r", 1);
    settled.settled = true;
    let mut snoozed = th("snoozed", "pa", "completed", "r", 1);
    snoozed.snoozed = true;
    set_list(&f, P1, Ok(vec![
        th("done", "pa", "completed", "r", 1),
        th("waiting", "pa", "waiting", "r", 1),
        th("running", "pa", "running", "r", 1),
        th("queued", "pa", "queued", "r", 1),
        th("inbox", "pa", "completed", "r", 1),
        settled,
        snoozed,
    ]));
    set_list(&f, P2, Ok(vec![]));
    db::insert_inbox(&c.db, "2026-11-06", "inbox", "now").unwrap();
    sync(&c).unwrap();
    let ids: Vec<String> = db::pending_threads(&c.db).unwrap().into_iter().map(|t| t.thread_id).collect();
    assert_eq!(ids, vec!["done", "waiting"]);
}

#[test]
fn fingerprint_ignores_updated_at_and_annotate_guards() {
    let mut f = FakeT3::default();
    f.messages.insert("t1".into(), vec![
        Message { role: "user".into(), text: "x".repeat(3000) },
        Message { role: "assistant".into(), text: "ok".into() },
    ]);
    let (c, f) = ctx(f);
    set_list(&f, P1, Ok(vec![th("t1", "pa", "completed", "run1", 4)]));
    set_list(&f, P2, Ok(vec![]));
    sync(&c).unwrap();

    let p = pending::pending(&c, None).unwrap();
    assert_eq!(p.threads.len(), 1);
    assert_eq!(p.threads[0].fingerprint, "run1:4");
    assert_eq!(p.threads[0].project, "a");
    assert_eq!(p.threads[0].messages[0].text.chars().count(), 2001); // 2000 + …

    // Stale fingerprint is discarded.
    let r = annotate::annotate(&c, annotate::parse(r#"[{"thread_id":"t1","fingerprint":"run1:3","kind":"todo","title":"x"}]"#).unwrap()).unwrap();
    assert_eq!(r.stale, vec!["t1"]);
    assert!(db::open_task_for_thread(&c.db, "t1").unwrap().is_none());

    // Matching fingerprint creates a task and clears pending.
    let r = annotate::annotate(&c, annotate::parse(r#"[{"thread_id":"t1","fingerprint":"run1:4","kind":"todo","topic":"移动端","title":"根因已定位","next":"修复"}]"#).unwrap()).unwrap();
    assert_eq!(r.created.len(), 1);
    assert!(db::pending_threads(&c.db).unwrap().is_empty());

    // Title change / settle-like updatedAt churn doesn't re-trigger (same run + items).
    let mut renamed = th("t1", "pa", "completed", "run1", 4);
    renamed.title = "renamed".into();
    set_list(&f, P1, Ok(vec![renamed]));
    sync(&c).unwrap();
    assert!(db::pending_threads(&c.db).unwrap().is_empty());

    // New activity → pending again → annotate updates the same task (upsert), keeps opened_at.
    set_list(&f, P1, Ok(vec![th("t1", "pa", "completed", "run2", 8)]));
    sync(&c).unwrap();
    assert_eq!(db::pending_threads(&c.db).unwrap().len(), 1);
    let before = db::open_task_for_thread(&c.db, "t1").unwrap().unwrap();
    let r = annotate::annotate(&c, annotate::parse(r#"{"thread_id":"t1","fingerprint":"run2:8","kind":"discuss","title":"要不要 B？","next":"选 A 还是 B？"}"#).unwrap()).unwrap();
    assert_eq!(r.updated, vec![before.id]);
    let after = db::task(&c.db, before.id).unwrap().unwrap();
    assert_eq!(after.kind, "discuss");
    assert_eq!(after.topic.as_deref(), Some("移动端"), "missing topic keeps the old one");
    assert_eq!(after.opened_at, before.opened_at);
}

#[test]
fn annotate_never_revives_closed_and_rejects_bad_kind() {
    let (c, f) = ctx(FakeT3::default());
    set_list(&f, P1, Ok(vec![th("t1", "pa", "completed", "r", 2)]));
    set_list(&f, P2, Ok(vec![]));
    sync(&c).unwrap();
    let id = task_for(&c, "t1", "todo");
    close::close(&c, &[id], false).unwrap();

    let r = annotate::annotate(&c, annotate::parse(r#"[{"thread_id":"t1","fingerprint":"r:2","kind":"bogus"},{"thread_id":"nope","fingerprint":"r:2","kind":"todo"}]"#).unwrap()).unwrap();
    assert_eq!(r.errors.len(), 2);

    let r = annotate::annotate(&c, annotate::parse(r#"[{"thread_id":"t1","fingerprint":"r:2","kind":"todo","title":"new"}]"#).unwrap()).unwrap();
    assert_eq!(r.created.len(), 1, "closed task stays closed, a new one is opened");
    assert_eq!(state(&c, id).0, "done");
}

#[test]
fn close_returns_threads_to_settle() {
    let (c, f) = ctx(FakeT3::default());
    let mut s = th("settled", "pa", "completed", "r", 1);
    s.settled = true;
    set_list(&f, P1, Ok(vec![th("live", "pa", "completed", "r", 1), s]));
    set_list(&f, P2, Ok(vec![]));
    sync(&c).unwrap();
    let a = task_for(&c, "live", "closeable");
    let b = task_for(&c, "settled", "todo");
    let note = add::add(&c, add::AddArgs { title: "note".into(), kind: "todo".into(), topic: None, project: None, thread: None, next: None }).unwrap();

    let r = close::close(&c, &[a, b, note, 999], true).unwrap();
    assert_eq!(r.closed, vec![a, b, note]);
    assert_eq!(r.state, "dropped");
    assert_eq!(r.threads_to_settle, vec![close::Settle { thread_id: "live".into(), title: "title live".into() }]);
    assert_eq!(r.skipped.len(), 1);
    assert!(f.calls.borrow().is_empty(), "close never calls T3");

    let again = close::close(&c, &[a], false).unwrap();
    assert!(again.closed.is_empty());
}

#[test]
fn open_sends_or_starts() {
    let (c, f) = ctx(FakeT3::default());
    set_list(&f, P1, Ok(vec![th("t1", "pa", "completed", "r", 1)]));
    set_list(&f, P2, Ok(vec![]));
    sync(&c).unwrap();
    let bound = task_for(&c, "t1", "todo"); // next = "do it"
    let note = add::add(&c, add::AddArgs { title: "深色模式".into(), kind: "todo".into(), topic: None, project: Some("/repo/a".into()), thread: None, next: Some("查对比度".into()) }).unwrap();
    let orphan = add::add(&c, add::AddArgs { title: "无项目".into(), kind: "todo".into(), topic: None, project: None, thread: None, next: None }).unwrap();

    let out = open::open(&c, &[bound, note], Some("选 B")).unwrap();
    assert_eq!(out[0], open::Opened::Sent { id: bound, thread_id: "t1".into() });
    assert!(matches!(&out[1], open::Opened::Started { id, .. } if *id == note));
    let calls = f.calls.borrow().clone();
    assert_eq!(calls[0], "send t1 do it\n\n选 B");
    assert_eq!(calls[1], "start /repo/a title=深色模式 model=-/- 深色模式\n\n查对比度\n\n选 B");
    let n = db::task(&c.db, note).unwrap().unwrap();
    assert_eq!(n.thread_id.as_deref(), Some("new-thread-2"));
    assert_eq!(n.state, "open");
    assert!(n.next.is_none());

    // Bound task with nothing to say → error, no call.
    assert!(open::open(&c, &[bound], None).is_err());
    // Note without project → error.
    assert!(open::open(&c, &[orphan], None).is_err());
    assert_eq!(f.calls.borrow().len(), 2);
}

#[test]
fn open_failed_send_keeps_next() {
    let (c, f) = ctx(FakeT3 { fail_send: true, ..Default::default() });
    set_list(&f, P1, Ok(vec![th("t1", "pa", "completed", "r", 1)]));
    set_list(&f, P2, Ok(vec![]));
    sync(&c).unwrap();
    let id = task_for(&c, "t1", "todo");
    assert!(open::open(&c, &[id], None).is_err());
    assert_eq!(db::task(&c.db, id).unwrap().unwrap().next.as_deref(), Some("do it"));
}

#[test]
fn tick_wakes_only_on_change_and_reuses_inbox() {
    let (c, f) = ctx(FakeT3::default());
    set_list(&f, P1, Ok(vec![]));
    set_list(&f, P2, Ok(vec![]));
    db::insert_inbox(&c.db, "2026-11-05", "old-inbox", "x").unwrap();
    assert_eq!(tick::tick(&c, false, "BODY").unwrap(), tick::Tick::NoChange);
    assert!(f.calls.borrow().is_empty());

    let r = tick::tick(&c, true, "BODY").unwrap();
    let tick::Tick::Woke { inbox, created: true, .. } = r else { panic!("{r:?}") };
    {
        let calls = f.calls.borrow();
        assert!(calls[0].starts_with("start inbox-proj title=📋 11/06 model=claudeAgent/haiku t3bot 巡检"), "{}", calls[0]);
        assert!(calls[0].contains("当天第一轮 · 昨天收件箱（2026-11-05）：old-inbox"));
        assert!(calls[0].ends_with("BODY"));
    }

    set_list(&f, P1, Ok(vec![th("t1", "pa", "completed", "r", 1)]));
    let r = tick::tick(&c, false, "BODY").unwrap();
    assert_eq!(r, tick::Tick::Woke { inbox: inbox.clone(), created: false, pending: 1 });
    let calls = f.calls.borrow();
    assert!(calls[1].starts_with(&format!("send {inbox} t3bot 巡检")));
    assert!(!calls[1].contains("当天第一轮"));
}

#[test]
fn edit_and_running_group() {
    let (c, f) = ctx(FakeT3::default());
    set_list(&f, P1, Ok(vec![th("t1", "pa", "running", "r", 1)]));
    set_list(&f, P2, Ok(vec![]));
    let r = sync(&c).unwrap();
    let id = task_for(&c, "t1", "todo");
    edit::edit(&c, id, edit::EditArgs { topic: Some("编排".into()), next: Some("".into()), ..Default::default() }).unwrap();
    let t = db::task(&c.db, id).unwrap().unwrap();
    assert_eq!(t.topic.as_deref(), Some("编排"));
    assert!(t.next.is_none());
    assert!(edit::edit(&c, id, edit::EditArgs { kind: Some("nope".into()), ..Default::default() }).is_err());
    let out = report::render(&c, &r).unwrap();
    assert!(out.contains("进行中：[#1](https://x/t1)"), "{out}");
    assert!(out.contains("待办 0"), "{out}");
}

#[test]
fn parse_ids_ranges() {
    let v = t3bot::parse_ids(&["3-5".into(), "#7,9".into()]).unwrap();
    assert_eq!(v, vec![3, 4, 5, 7, 9]);
    assert!(t3bot::parse_ids(&["x".into()]).is_err());
}

// ---- setup / doctor ----------------------------------------------------------

use t3bot::cmd::setup::{self, Level};

#[test]
fn doctor_flags_missing_inbox_and_broken_projects() {
    let fake = t3bot::t3::FakeT3::default();
    fake.lists.borrow_mut().insert("/ok".into(), Ok(vec![]));
    let cfg = Config { projects: vec!["/ok".into(), "/gone".into()], provider: Some("p".into()), ..Default::default() };
    let checks = setup::check_t3(&cfg, &fake);
    let level = |what: &str| checks.iter().find(|c| c.what == what).map(|c| c.level);
    assert_eq!(level("inbox_project"), Some(Level::Fail));
    let project = |path: &str| checks.iter().find(|c| c.what == "project" && c.detail.starts_with(path)).map(|c| c.level);
    assert_eq!(project("/ok"), Some(Level::Ok));
    assert_eq!(project("/gone"), Some(Level::Fail));
    assert_eq!(level("inbox model"), Some(Level::Warn));
}

#[test]
fn doctor_ok_and_warns_when_inbox_is_swept() {
    let fake = t3bot::t3::FakeT3::default();
    fake.lists.borrow_mut().insert("inbox-id".into(), Ok(vec![]));
    let cfg = Config { inbox_project: Some("inbox-id".into()), projects: vec!["inbox-id".into()], ..Default::default() };
    let checks = setup::check_t3(&cfg, &fake);
    assert!(checks.iter().all(|c| c.level != Level::Fail), "{checks:?}");
    assert!(checks.iter().any(|c| c.level == Level::Warn && c.what == "inbox_project"));
}

#[test]
fn init_adds_projects_once_and_rm_removes() {
    let dir = std::env::temp_dir().join(format!("t3bot-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let abs = std::fs::canonicalize(&dir).unwrap().to_string_lossy().into_owned();
    let mut cfg = Config::default();
    let args = || setup::InitArgs { projects: vec![abs.clone(), "some-project-id".into()], ..Default::default() };
    setup::apply_init(&mut cfg, args()).unwrap();
    setup::apply_init(&mut cfg, args()).unwrap();
    assert_eq!(cfg.projects, vec![abs.clone(), "some-project-id".to_string()]);
    assert!(setup::add_projects(&mut cfg, &["/definitely/not/here".into()]).is_err());
    setup::rm_projects(&mut cfg, &[abs.clone()]).unwrap();
    assert_eq!(cfg.projects, vec!["some-project-id".to_string()]);
    assert!(setup::rm_projects(&mut cfg, &[abs]).is_err());
    std::fs::remove_dir_all(&dir).unwrap();
}
