---
name: t3bot
description: Follow-up ledger for T3 threads. Use ONLY when the user's message starts with `@t3bot` (e.g. "@t3bot 记一下 …", "@t3bot 关 3-5", "@t3bot 开 2", "@t3bot 看看"). Records notes and closes/starts follow-ups with the `t3bot` CLI. Do not use for messages without the `@t3bot` prefix.
---

# t3bot

`t3bot` is a CLI on this machine (`~/.local/bin/t3bot`) that keeps a ledger of unfinished
follow-ups across T3 threads. Every item has a stable number `#N` (the same in every report).
Only act when the user's message starts with `@t3bot`. Run the commands with the shell, never
pass `--wait` to anything, and report back in one or two lines.

## Commands

| User says | Run |
|---|---|
| `@t3bot 记一下 [主题]：内容` | `t3bot add "内容" --topic 主题 --kind todo --project <current project path>` |
| … a question/decision rather than a task | same, with `--kind discuss` |
| … about *this* thread | add `--thread <current thread id>` (only if you know it) |
| `@t3bot 关 3-5` / `丢 3` | `t3bot close 3-5 --json` / `t3bot close 3 --drop --json`, then settle (below) |
| `@t3bot 开 2` / `开 2：选方案 B` | `t3bot open 2` / `t3bot open 2 --say "选方案 B"` |
| `@t3bot 改 4 主题→移动端` | `t3bot edit 4 --topic 移动端` (also `--kind`, `--title`, `--next`, `--project`) |
| `@t3bot 看看` / `列表` | `t3bot ls` (or `t3bot report` for the grouped report) |

- `--project` takes a path inside the project (your working directory is fine) or a project
  id. It is needed so `t3bot open` can later start a thread for the note.
- Kinds: `todo` (an agent can do the next step), `discuss` (needs the user's decision),
  `closeable` (done / pure Q&A).
- `t3bot open N` sends the item's `next` (plus `--say`) to its thread, or starts a new thread
  for a note. It returns immediately; the target thread runs on its own.

## After `close`: settle the threads

`t3bot close … --json` prints `threads_to_settle`. The CLI cannot settle threads yet, so for
each entry call the T3 MCP tool:

```
t3_thread_organize(action: "settle", threadId: "<thread_id>")
```

Skip this step if the list is empty. Then tell the user what was closed and settled.
