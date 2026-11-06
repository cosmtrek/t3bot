# t3bot

A follow-up ledger for [T3 Code](https://t3.codes) threads. A thread showing `completed` only means
one turn ended. t3bot records, for every unfinished item, what the next step is and whose move it is,
and posts a grouped report (待办 / 待议 / 可关) with stable `#numbers` into a daily 📋 inbox thread,
where one reply such as `关 3-5；开 1；2 选方案 B` acts on them.

It is not an orchestrator: execution and queuing stay in T3. t3bot only calls the installed `t3` CLI
(`thread list / show / start / send`, never `--wait`). Classification is done by the inbox agent,
not by direct LLM API calls. Design: [`docs/design.md`](docs/design.md).

## Install

Linux (x86_64 / aarch64), on the machine that runs the T3 server:

```sh
curl -fsSL https://raw.githubusercontent.com/cosmtrek/t3bot/main/scripts/install.sh | sh
```

This installs `~/.local/bin/t3bot` from the latest release (checksum-verified), the `@t3bot`
skill into `~/.claude/skills`, `~/.agents/skills` and `~/.gemini/antigravity-cli/skills`, and
systemd user timers: 09:00 `tick --always`, 13:30 and 21:00 `tick`, in the machine's local
time. On a fresh install it then runs `t3bot init`, which asks for the inbox project and the
projects to sweep, and `t3bot doctor`.

Options: `sh -s -- --version v0.2.0`, `--no-systemd`, `--force`; `T3BOT_BIN_DIR` changes the
install directory. From a checkout: `./scripts/install.sh --from-source`.

## Upgrade

```sh
t3bot upgrade                 # latest release; or --version vX.Y.Z
```

This reruns the installer into the directory `t3bot` lives in. The binary, skill and timers are
replaced; `~/.t3bot/config.toml`, `prompt.md` and `state.db` are kept.

## Configure

`~/.t3bot/config.toml` (or `$T3BOT_HOME/config.toml`) holds the inbox project, the inbox
model, the projects to sweep and, if needed, the `t3` path. You can edit it by hand or use:

```sh
t3bot init --inbox ~/inbox --project ~/code/app --project ~/code/api \
           --provider antigravity --model gemini-3.8-flash   # create/update; safe to rerun
t3bot config                        # show path + contents
t3bot config set model gemini-3.8-flash      # inbox_project | provider | model | t3_bin
t3bot config add-project ~/code/web          # rm-project to stop sweeping one
t3bot doctor                        # check config, t3, every project, skill, timers, linger
```

Paths may start with `~/` and are stored that way, so the same file works on another machine
with the same layout. `init` pins `t3_bin` automatically when `t3` is not on the PATH the
timers use. Override the wake prompt with `~/.t3bot/prompt.md` (`t3bot init --prompt` prints
the built-in one).

## Commands

| Command | What it does |
|---|---|
| `t3bot add <title> [--kind todo\|discuss] [--topic T] [--project P] [--thread ID] [--next TEXT]` | Record a follow-up or note |
| `t3bot ls [--topic T] [--all] [--json]` | List open (or all) items |
| `t3bot edit <id> [--kind K] [--topic T] [--title S] [--next TEXT] [--project P]` | Change an item |
| `t3bot open <ids> [--say TEXT]` | Send `next` (+ say) to the item's thread, or start a thread for a note |
| `t3bot close <ids> [--drop] [--json]` | Mark done/dropped; prints `threads_to_settle` for the agent to settle via MCP |
| `t3bot report [--json]` | Sync and render the report |
| `t3bot pending --json [--limit N]` | Threads that need (re)classification, with their last messages |
| `t3bot annotate -` | Store classification JSON from stdin (fingerprint-checked) |
| `t3bot tick [--always]` | Timer entry: sync, and wake today's inbox if anything changed |
| `t3bot sync` | Sync only |
| `t3bot init` / `config` / `doctor` / `upgrade` | Setup, see above |

Ids accept `3 4 5`, `3-5` or `#3,4`.

## Development

```sh
cargo test
```

CI runs the tests and a [gitleaks](https://github.com/gitleaks/gitleaks) secret scan on every
push and pull request.

## Versions and releases

You pick the version; nothing updates on its own.

- **Release builds** (from a tag, built by CI) report a plain version: `t3bot 0.2.0`.
- **Local builds** (`cargo build`, `install.sh --from-source`) report
  `0.2.0-dev+<commit>[.dirty]`, so `t3bot --version` and `t3bot doctor` show which one runs.
- **Machines upgrade when you run `t3bot upgrade`.** It replaces a dev build too, without
  `--force`. `t3bot doctor` says when a newer release exists.

Releases are cut with [release-plz](https://release-plz.dev) from
[conventional commits](https://www.conventionalcommits.org) (`feat:`, `fix:`, `docs:`, …):

1. Every push to `main` opens or updates a **release PR**. It bumps `Cargo.toml` and adds the
   new section to [`CHANGELOG.md`](CHANGELOG.md). Before 1.0, `feat:` bumps the minor
   (0.1.0 → 0.2.0), `fix:` and the rest bump the patch, and `feat!:` / `BREAKING CHANGE` bump
   the minor.
2. Review it; edit the version or changelog in the PR if needed.
3. **Merge it** to release. The workflow tags `vX.Y.Z` and creates the GitHub release with the
   changelog. It then builds static musl binaries for `x86_64` and `aarch64` Linux and
   attaches `t3bot-<tag>-<target>.tar.gz` (binary, skill, systemd units) with SHA-256
   checksums.

Nothing is released until a release PR is merged. To rebuild the binaries of an existing tag,
run the `build` workflow by hand. Configuration: [`release-plz.toml`](release-plz.toml).

## License

[MIT](LICENSE)
