# t3bot 技术方案

## 1. 问题

在 T3 里，一个 thread 显示 `completed`，只代表"这一轮回复结束了"，不代表"这件事做完了"。agent 停下以后，"下一步做什么、现在该谁动"这件事，在 T3 里没有地方记，也没有人去推。具体表现为三种：

- **待办没人推**：根因或方案已经出来，agent 完全可以接着做，但没有人发出下一条消息。
- **待议没处放**：需要我拍板的问题、随手冒出来的想法，留在聊天记录里很快就沉底。
- **可关的没收起**：纯问答、已经交接完的 thread 堆在列表里，把真正需要关注的事项淹没了。

## 2. 目标与非目标

**目标**

1. 每天固定几个时间点，在一个收件箱 thread 里看到所有没闭环的事项，按"待办 / 待议 / 可关"分组列出，编号保持稳定。
2. 回一句话就能批量处理，比如"关 3-5；开 1；2 选方案 B"，动作直接落到 T3 上（收起 thread、发消息）。
3. 在任何 thread 里都能随手记一笔（`@t3bot 记一下…`）。
4. 没有变化时不消耗 token。

**非目标**（这些都是有意不做的）

- **t3bot 不当编排者**：不做排队、重试、盯运行、管依赖。执行和排队由 T3 负责，T3 本身就是事件溯源的编排器，同一个 thread 的消息也原生排队。
- **不复制 T3 的事实数据**：标题、消息、运行状态以 T3 为准，t3bot 只缓存做变化检测需要的那一点。
- 不做常驻 daemon，不直连 LLM API，不支持多机，不做报告模板引擎。

## 3. 核心决策

| # | 决策 | 理由 |
|---|---|---|
| D1 | **t3bot 是"跟进账本"，不是编排者。** 它只记录每件事的下一步、现在该谁动。 | 一旦当编排者，就要在 T3 外面再镜像一份运行状态机，然后靠轮询去对账。水位误触发、settled 归因、派发幂等这类问题会接连出现，根源就是有两份状态。 |
| D2 | **t3bot 永远不用 `--wait`。** 它对 T3 的写操作都是一次性、非阻塞的：`send`、`start`、`organize`。 | 不阻塞就不会出现 Agent→t3bot→Agent 的循环。这样就不用按"谁在调用"把命令拆成两套，同一个命令在终端、cron、agent 里调用，行为都一样。 |
| D3 | **现有 CLI 能做的，t3bot 自己去调 T3**："开 #2"由 t3bot 直接 send 或 start。"关 #3"时的收起 thread，CLI 补上 `organize` 之前先由 agent 调 MCP 完成（见 §4）。 | 能确定的步骤尽量写进代码，agent 少执行一步，便宜模型就少一个漏步骤的机会。 |
| D4 | **分类由收件箱 agent 在被唤醒的那一轮里完成。** 不另开分析 thread，也不直连 API。 | 只有一条 LLM 路径。分类和汇报在同一轮完成，只计费一次。用哪个模型，由收件箱 thread 的模型决定。 |
| D5 | **水位用 `latestRunId:itemCount` 当指纹，不用 `updatedAt`。** | settle、pin、改标题都会刷新 `updatedAt`，但这些不代表内容变了。 |
| D6 | **报告里的编号直接用 `tasks.id`。** | 早报里的 #12 到晚报还是 #12，照着旧消息回复也不会关错。 |
| D7 | **定时交给系统定时器，t3bot 里不写时间槽逻辑。** 本机没有 cron 守护进程，用 systemd user timer（root 已开 linger，T3 自己也是 user service）。 | 定时器本来就擅长定时，t3bot 只负责"现在跑一次"。 |

## 4. 只依赖现有的 T3 CLI

t3bot 只用已安装的 `t3`（0.1.7）里已有的命令：`thread list / show / start / send`。现有 CLI 缺两样东西，用下面的办法绕开：

| 缺的能力 | 现在的做法 |
|---|---|
| 列出所有项目（没有 `project list`，也没有 `thread list --all-projects`） | 在 `config.toml` 里写 `projects = ["/path/to/project", …]`，逐个执行 `t3 thread list --project <路径> --json`（CLI 内部已经翻完所有分页）。`--project` 本来就接受项目内的路径。路径失效时 stderr 警告，报告末尾也会带一行 ⚠️。 |
| 收起 thread（没有 `thread organize`） | 收起只会发生在两个地方：一是我在收件箱回复"关 N"，二是收起昨天的收件箱。这两个时刻收件箱 agent 都在跑，手上有 MCP 的 `t3_thread_organize`。所以 `close --json` 返回 `threads_to_settle`，由 agent 去收起；收起昨天收件箱这一步写进唤醒 prompt。 |

**后续优化（不阻塞使用）**：给 t3code 提 PR，补上 `t3 thread list --all-projects` 和 `t3 thread organize`。服务端的 MCP 工具已经有了，只差 CLI 参数。等新版本发布、本机升级以后，删掉上面两条绕行和 `projects` 配置。

**不要**用源码里的 CLI 去连正在用的安装：CLI 会直接在 `~/.t3/userdata` 里签发会话，源码版本和已安装版本不一致时，有改坏线上数据库的风险。

不补 `--client-request-id`。理由见 §7 的派发说明。

## 5. 数据模型（SQLite，3 张表）

```sql
-- 跟进事项。一行就是"一件还没闭环的事"，可能绑定 thread，也可能是随手记。
CREATE TABLE tasks (
  id          INTEGER PRIMARY KEY,           -- 就是报告里的 #编号
  thread_id   TEXT,                          -- NULL = 随手记，还没开工
  project_id  TEXT,                          -- 随手记开工时需要；绑定 thread 时从 thread 继承
  kind        TEXT NOT NULL,                 -- todo | discuss | closeable
  state       TEXT NOT NULL DEFAULT 'open',  -- open | done | dropped
  topic       TEXT,
  title       TEXT NOT NULL,                 -- 一句话现状
  next        TEXT,                          -- todo：可以直接发出的指令；discuss：要我拍板的问题
  source      TEXT NOT NULL,                 -- sweep | manual
  close_note  TEXT,                          -- dropped 的原因，例如 thread_gone
  opened_at   TEXT NOT NULL,                 -- 用来算"已卡 N 天"，重新分类时不重置
  updated_at  TEXT NOT NULL,
  closed_at   TEXT
);
CREATE UNIQUE INDEX one_open_task_per_thread
  ON tasks(thread_id) WHERE state = 'open' AND thread_id IS NOT NULL;

-- T3 thread 的缓存，只用于变化检测和报告展示。T3 才是真相来源。
CREATE TABLE threads (
  thread_id            TEXT PRIMARY KEY,
  project_id           TEXT NOT NULL,
  project_path         TEXT NOT NULL,        -- 列出它的 config 路径，用来判断 gone（见同步规则）
  title                TEXT NOT NULL,
  link                 TEXT,                 -- T3 返回的 link，报告里给编号挂链接
  status               TEXT NOT NULL,        -- T3 的 status 原值
  settled              INTEGER NOT NULL,
  snoozed              INTEGER NOT NULL,
  fingerprint          TEXT NOT NULL,        -- "<latestRunId>:<itemCount>"
  analyzed_fingerprint TEXT,                 -- 上次分类时的指纹，NULL 表示从未分类
  gone_at              TEXT                  -- 在 T3 里已经找不到
);

-- t3bot 自己建的收件箱 thread，也用来把它们排除在巡检之外。
CREATE TABLE inboxes (
  date       TEXT PRIMARY KEY,               -- 本地日期 2026-11-06
  thread_id  TEXT NOT NULL,
  created_at TEXT NOT NULL
);
```

**有意不建的表**

- **projects**：项目列表写在 `config.toml` 里，不需要建表。等有了 `--all-projects`，连这项配置也可以删掉。项目被删除后，它下面的 thread 会从列表里消失，按 thread 消失的规则处理即可。
- **topics**：候选主题直接用 `SELECT DISTINCT topic FROM tasks WHERE state='open'`。一个主题下的事项全部关掉，它就自然从候选里消失。
- **历史表**：`opened_at` 已经够算"卡了几天"，用不到更多历史。

**同步规则**（`sync`，每个读命令开始时执行）

1. 对 config 里的每个项目调 `t3 thread list --project <路径> --json`，按 thread_id upsert 到 `threads`（重新出现的清掉 `gone_at`）。缓存里 `project_path` 属于列表成功的项目、但这次没出现的 thread 标 `gone_at`；`project_path` 已经不在 config 里的也标 gone。如果某个项目整个列表都拿不到，跳过这个项目，**不要**把它名下的 thread 都标成 gone。之所以按 `project_path` 而不是 `project_id` 判断，是因为列表为空时拿不到项目 id。
2. thread 消失了：它名下 open 的任务改成 `dropped`，`close_note='thread_gone'`。
3. thread 变成 settled：只自动关闭 `kind='closeable'` 的任务。todo 和 discuss 保持 open，报告里标"📦 已收起未闭环"。原因是 T3 有自动收起，不能因为收起就默认事情做完了。

**待分类条件**（`pending`）

```
fingerprint != analyzed_fingerprint
AND status NOT IN (preparing, queued, starting, running)   -- waiting 要算进来，它就是"等我回话"
AND NOT settled AND NOT snoozed
AND thread_id NOT IN (SELECT thread_id FROM inboxes)
```

**并发**：这是单人使用的工具，不加 rev 或 CAS。唯一一道防线是：`annotate` 提交的指纹必须等于 `threads.fingerprint`，否则丢弃这条结果，等下一轮重新分类。分类只改 open 任务的 kind、topic、title、next，从不改 state，所以人工关掉的事项不会被分类器"复活"。如果 thread 之后又有新对话，会新开一个任务。

## 6. CLI

```text
t3bot add <title> [--kind todo|discuss] [--topic T] [--project P] [--thread ID] [--next TEXT]
t3bot ls [--topic T] [--all] [--json]
t3bot edit <id> [--kind K] [--topic T] [--title S] [--next TEXT]
t3bot open <id>... [--say TEXT]          # 开工：把 next（加上 --say）发出去
t3bot close <id>... [--drop] [--json]    # done/dropped，返回 threads_to_settle，由 agent 去收起
t3bot report [--json]                    # 渲染当前报告，只读
t3bot pending --json [--limit N]         # 同步，然后输出待分类的 thread 和上下文
t3bot annotate -                         # 从 stdin 读分类结果，按指纹校验后写入
t3bot tick [--always]                    # 定时器入口
t3bot sync                               # 只同步，打印 seen/gone/pending 计数（排查用）
t3bot init [--prompt]                    # 打印示例 config / 内置唤醒 prompt
```

编号参数都接受 `3 4 5`、`3-5`、`#3,4` 这几种写法。`edit` 还可以改 `--project`（随手记开工前补项目），传空串表示清空。

**`open` 的行为**

- 任务已经绑定 thread：执行 `t3 thread send <tid> "<next>\n\n<say>"`。thread 正忙也没关系，T3 会把消息排进队列。
- 任务是随手记：执行 `t3 thread start <project> - --title <title> --json`（正文 `title + next + say` 走 stdin），再把返回的 threadId 回填到任务上。没有 project 就报错退出。不传 `--provider/--model`，用 T3 对该项目的默认模型（项目没设时用服务端的兜底模型，实测是 codex）。
- 开工后任务保持 `open`，`next` 清空。thread 停下来以后，下一轮巡检会给同一个任务重新分类。thread 正在跑时，报告把它放进"进行中"分组，不当作待处理事项。

**`pending --json` 输出**（给收件箱 agent 读）

```json
{
  "topics": ["编排", "移动端"],
  "threads": [{
    "thread_id": "…", "fingerprint": "run_x:42", "title": "…", "status": "completed",
    "project": "t3code",
    "open_task": { "id": 12, "kind": "todo", "title": "…", "next": "…" },
    "messages": [{ "role": "user", "text": "…" }, { "role": "assistant", "text": "…" }]
  }]
}
```

`messages` 取自 `t3 thread show <id> --messages 4 --json`，每条截到 2000 字。`--limit` 让便宜模型分批处理：已经 annotate 的不会再出现，所以 agent 循环"取 5 个 → annotate"直到为空即可（实测一次吞下 12 个 thread、约 4 万字时，Gemini Flash 只认真读了前 5 个）。

**`annotate` 输入**

```json
[{ "thread_id": "…", "fingerprint": "run_x:42",
   "kind": "todo", "topic": "移动端", "title": "链接失效根因已定位", "next": "按根因修复并补测试" }]
```

## 7. 核心流程

**A. 定时巡检**（systemd user timer 在 09:00 / 13:30 / 21:00 执行 `t3bot tick`，09:00 那次加 `--always`）

```
timer → t3bot tick（fd-lock 防重入）
        ├─ sync（t3 thread list，零 token）
        ├─ 没有待分类的 thread，也没有 --always → 退出
        ├─ 今天还没有收件箱 → t3 thread start <inbox_project> - --title "📋 11/06"
        │                       --provider antigravity --model gemini-3.8-flash
        │                     （首条消息就是唤醒 prompt，不再另外 send）
        └─ 已有收件箱 → t3 thread send <今天的收件箱> -       （都不 --wait，执行完就退出）
收件箱 agent（便宜模型）
        ├─ 如果是当天第一轮：用 t3_thread_organize 收起昨天的收件箱
        ├─ 循环 t3bot pending --json --limit 5 → 判断 → t3bot annotate -，直到为空
        └─ t3bot report → 原样贴出
```

唤醒消息 = t3bot 生成的一行本轮信息（时间、待分类数、是否当天第一轮、昨天收件箱 id、列表失败的项目）+ prompt 正文。正文默认内置在二进制里（源文件 `prompts/wake.md`），可以用 `~/.t3bot/prompt.md` 整体覆盖。分类规则和展示偏好都写在正文里，改完下一轮就生效，不需要重新编译。

**收件箱的运行模式（实测）**：`t3 thread start` 没有 runtime-mode 参数，新 thread 的模式 = CLI 调用方的上限（full-access）再被"项目默认运行模式"（T3 设置里的 `defaultRuntimeMode`，可按项目覆盖；没有 CLI/MCP 能改）收窄。收件箱项目没设，所以收件箱是 full-access。这带来两个约束：

- Claude 在 full-access 下会带 `--dangerously-skip-permissions`，T3 以 root 运行时 Claude 直接拒绝启动（run 立刻 failed）。所以收件箱默认用 `antigravity / gemini-3.8-flash`，它在 full-access 下正常执行 shell 和 MCP，不需要任何审批。
- 收件箱**需要** full-access：MCP 不允许低模式的 thread 收起比它模式高的 thread（实测 auto 去 settle full-access thread 报 `runtime_mode_escalation_denied`），而 Antigravity 的 thread 都是 full-access。所以"把收件箱项目默认模式改成 auto、收件箱换 Haiku"虽然能跑 t3bot 命令（实测 Haiku + auto 执行 `t3bot` 不需审批），但关不掉 full-access 的 thread，不采用。

**B. 批阅**（我在收件箱里回"关 3-5；开 1；2 选方案 B 直接改"）

```
收件箱 agent → t3bot close 3 4 5 --json → 用 t3_thread_organize 收起 threads_to_settle 里的 thread
             → t3bot open 1
             → t3bot open 2 --say "选方案 B，直接改"
```

**C. 随手记**（在任意 thread 里说"@t3bot 记一下 [移动端]：深色模式代码块对比度"）

```
当前 agent → t3bot add "深色模式代码块对比度" --topic 移动端 --kind todo --project <当前项目>
```

agent 是通过一个全局安装的 skill（`t3bot/SKILL.md`）知道该怎么调用的。只认 `@t3bot` 前缀，避免误触。

**派发不需要幂等键**：`open` 是我手动触发的同步命令，顺序是先 send、成功后再写 DB。只有在 send 成功、写 DB 失败的瞬间才会出问题，后果是重试时消息发了两条，而且我当场就能看到。为这个窗口专门引入 dispatching 状态和 client-request-id，不值得。

## 8. 报告格式（写死在代码里，保证稳定）

```
📋 11/06 13:30   待办 3 · 待议 2 · 可关 4

[移动端]
- [#12](link) ▶ 链接失效根因已定位（停 18h） → 按根因修复并补测试
- [#15](link) ❓ 深色模式要不要单独一套代码高亮主题？（已议 3 天）

[编排]
- [#9](link) ▶ 📦 已收起未闭环：t3 CLI 补 organize 子命令（停 2 天）
- #17 ▶ 📝 深色模式代码块对比度（停 5h）

进行中：[#7](link) [#11](link)

可关：
- [#3](link) 纯问答已答完
- [#4](link) 已交接给新 thread

待分类：2 个 thread（下一轮巡检处理）
⚠️ 项目列表失败：/path/to/project（…）
```

报告是 markdown：T3 会把相邻的普通行合并成一段，所以条目用列表，编号本身就是 thread 链接。📝 = 随手记（还没有 thread），⏳ = thread 在 waiting，📦 = 已收起未闭环。时长按 `opened_at` 算，不到 1 小时显示"刚刚"。可关一栏带上一句话，方便直接决定。没有分组主题的放在 `[其他]`。如果需要改格式，在 `prompt.md` 里让 agent 在报告前后加内容，报告本体保持不变。

## 9. 实现

- **技术栈**：Rust 单二进制。`clap`（derive）、`rusqlite`（bundled，WAL 模式）、`serde` / `serde_json`、`fd-lock`、`jiff`（本地日期/时区）、`anyhow`。不需要 async runtime，调 T3 用 `std::process::Command` 执行 `t3 … --json`，消息正文一律走 stdin（`-`）。`config.toml` 只用到字符串和字符串数组，手写了一个几十行的小解析器，没有引入 toml crate。
- **结构**：
  - `t3.rs`：把 T3 的调用收在一个 `trait T3`（list / show / send / start）里，测试时用假实现替换。0.1.7 的 CLI 没有 organize，所以 trait 里也没有，等 CLI 补上再加。
  - `db.rs`：放迁移和查询。
  - `sync.rs`：放 §5 的同步规则。
  - `cmd/*.rs`：每个子命令一个文件。
- **目录**：`~/.t3bot/state.db`、`~/.t3bot/config.toml`（收件箱所在项目、provider、model、要巡检的项目、可选 `t3_bin`）、`~/.t3bot/prompt.md`（可选）、`~/.t3bot/tick.lock`。`T3BOT_HOME` 可以换目录。
- **安装 / 升级**：`scripts/install.sh` 默认从 GitHub release 下载对应架构的静态二进制（校验 SHA-256），`--from-source` 则本地编译；装到 `~/.local/bin/t3bot`，把 `skill/t3bot/SKILL.md` 拷到 `~/.claude/skills/`、`~/.agents/skills/`、`~/.gemini/antigravity-cli/skills/`，装上 `contrib/systemd/` 里的 timer。首次安装跑 `t3bot init`（交互填收件箱和巡检项目），之后跑 `t3bot doctor`。`t3bot upgrade` 重新执行安装脚本，配置和数据不动。
- **发布**：在 `main` 上的提交打 `vX.Y.Z` tag 并推送，`release` workflow 校验 tag 在 main 上且与 `Cargo.toml` 一致，跑测试，构建 x86_64 / aarch64 musl 二进制并发布 release。
- **配置**：`t3bot init` 可重复执行，路径统一存成 `~/…`；`t3` 不在 timer 的 PATH 里时自动写 `t3_bin`。`t3bot config set|add-project|rm-project` 改配置，`t3bot doctor` 逐项检查（配置、t3、每个项目、收件箱、skill、timer、linger）。
- **测试**：只测会出错的地方，即同步规则（settled、gone、某项目列表失败、指纹）、pending 筛选条件、annotate 的指纹校验与 upsert、open 和 close 对 T3 发出的调用、tick 是否唤醒及收件箱复用。用内存 SQLite 加假的 T3 实现（`tests/logic.rs`）。
- **仓库**：`cosmtrek/t3bot`。

## 10. 分期

| 阶段 | 内容 | 完成后可用 |
|---|---|---|
| P1 | `add / ls / edit / close / report` 和 skill | 随手记、手动看板，不涉及 LLM |
| P2 | `sync / pending / annotate / tick`、systemd timer、prompt | 定时巡检，替换现在的 3 个 schedule_task |
| P3 | `open` | 在收件箱一句话派活 |
| 并行 | 给 t3code 提 PR：`list --all-projects`、`thread organize`。新版本到达本机后删掉 §4 的绕行 | 不阻塞 P1 到 P3 |
| 以后 | 只有出现真实需求才做，例如"#1 验收通过后再跑 #4"这种跨 thread 依赖 | 先评估能不能做成 T3 原生能力，再考虑在 t3bot 里实现 |

## 11. 风险与待确认

1. **收件箱的运行模式**（已实测，见 §7 A）：CLI 建出来的是 full-access，Claude 在 root 下拒绝启动，所以收件箱用 Gemini Flash。要换回 Claude，得先让 T3 不以 root 运行，或者 CLI 支持指定 runtime mode 且 MCP 放宽"低模式不能收起高模式"。同样的约束也落在 `open` 随手记上：如果某个项目的默认模型是 Claude，`open` 在那里建的 thread 会立刻失败。
2. **收件箱项目**：不用 "No project"，用一个不在巡检列表里的现成项目（例如一个 scratch 项目）。注意 scratch 类项目里每个新 thread 可能会建一个子目录。
5. **旧的 3 个定时巡检任务还在**：旧的 09:00 开新任务会把"除自己外最新的 📋 thread"取消置顶并收起，和 t3bot 的 `📋 MM/DD` 收件箱撞名。两套并存时，t3bot 的收件箱可能在 09:00 被旧任务收起。替换前要停掉旧任务。
3. **分类质量**：只看最后 4 条消息，可能判断不准。先跑一周，再调消息条数和 prompt。
4. **t3bot 必须和 T3 server 跑在同一台机器上**，因为 `t3` CLI 连接的是本机 server。手机通过收件箱 thread 交互，不受影响。
