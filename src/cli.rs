use crate::cmd::{self, add::AddArgs, edit::EditArgs, open::Opened, setup, setup::InitArgs, tick::Tick};
use crate::{Ctx, config, db, parse_ids, sync, t3};
use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::io::Read;

#[derive(Parser)]
#[command(name = "t3bot", version, about = "Follow-up ledger for T3 threads: what's next, and whose move it is.")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Record a follow-up (a note, or bound to a thread with --thread).
    Add {
        title: String,
        #[arg(long, default_value = "todo")]
        kind: String,
        #[arg(long)]
        topic: Option<String>,
        /// Project id or path; needed later by `open` for a note without a thread.
        #[arg(long)]
        project: Option<String>,
        #[arg(long)]
        thread: Option<String>,
        #[arg(long)]
        next: Option<String>,
    },
    /// List open tasks.
    Ls {
        #[arg(long)]
        topic: Option<String>,
        /// Include closed tasks.
        #[arg(long)]
        all: bool,
        #[arg(long)]
        json: bool,
    },
    /// Change a task. An empty string clears topic/next/project.
    Edit {
        id: i64,
        #[arg(long)]
        kind: Option<String>,
        #[arg(long)]
        topic: Option<String>,
        #[arg(long)]
        title: Option<String>,
        #[arg(long)]
        next: Option<String>,
        #[arg(long)]
        project: Option<String>,
    },
    /// Start work: send `next` (+ --say) to the task's thread, or start a thread for a note.
    Open {
        #[arg(required = true)]
        ids: Vec<String>,
        #[arg(long)]
        say: Option<String>,
    },
    /// Mark done (or --drop). Prints threads_to_settle for the agent to settle via MCP.
    Close {
        #[arg(required = true)]
        ids: Vec<String>,
        #[arg(long)]
        drop: bool,
        #[arg(long)]
        json: bool,
    },
    /// Sync and render the report.
    Report {
        #[arg(long)]
        json: bool,
    },
    /// Sync and print threads that need (re)classification, with context.
    Pending {
        #[arg(long)]
        json: bool,
        #[arg(long)]
        limit: Option<usize>,
    },
    /// Read classification JSON (array) from stdin and store it.
    Annotate {
        /// Must be "-" (stdin).
        #[arg(default_value = "-")]
        input: String,
    },
    /// Cron entry: sync; wake today's inbox if anything changed (or --always).
    Tick {
        #[arg(long)]
        always: bool,
    },
    /// Sync only, and print what happened.
    Sync,
    /// Create or update ~/.t3bot/config.toml (asks for missing values in a terminal), then run doctor.
    Init {
        /// Inbox project: a path inside it, or its id.
        #[arg(long)]
        inbox: Option<String>,
        /// Projects to sweep (repeatable; added to the existing list).
        #[arg(long = "project", value_name = "PATH")]
        projects: Vec<String>,
        #[arg(long)]
        provider: Option<String>,
        #[arg(long)]
        model: Option<String>,
        /// Never prompt.
        #[arg(long)]
        no_input: bool,
        /// Print an example config instead of writing one.
        #[arg(long, conflicts_with = "prompt")]
        print: bool,
        /// Print the built-in wake prompt (copy to ~/.t3bot/prompt.md to override it).
        #[arg(long)]
        prompt: bool,
    },
    /// Show or change the config.
    Config {
        #[command(subcommand)]
        cmd: Option<ConfigCmd>,
    },
    /// Check the config, T3, projects, skill and timers.
    Doctor,
    /// Upgrade to the latest release (or --version): binary, skill and timers.
    Upgrade {
        #[arg(long)]
        version: Option<String>,
        /// Reinstall even if already on that version.
        #[arg(long)]
        force: bool,
    },
}

#[derive(Subcommand)]
enum ConfigCmd {
    /// Print the config file path and contents (default).
    Show,
    /// Print the config file path.
    Path,
    /// Set inbox_project, provider, model or t3_bin.
    Set { key: String, value: String },
    /// Clear a key.
    Unset { key: String },
    /// Add projects to sweep.
    AddProject {
        #[arg(required = true)]
        paths: Vec<String>,
    },
    /// Stop sweeping projects.
    RmProject {
        #[arg(required = true)]
        paths: Vec<String>,
    },
}

fn ctx() -> Result<Ctx> {
    let home = config::home();
    std::fs::create_dir_all(&home).with_context(|| format!("creating {}", home.display()))?;
    let path = config::path();
    if !path.exists() {
        eprintln!("t3bot: warning: no config at {} → run `t3bot init`", path.display());
    }
    let cfg = config::Config::load(&path)?;
    Ok(Ctx {
        db: db::open(&home.join("state.db"))?,
        t3: Box::new(t3::CliT3 { bin: cfg.t3_bin() }),
        cfg,
        now: jiff::Zoned::now(),
    })
}

/// Print doctor checks. Returns false if any check failed.
fn doctor(cfg: &config::Config, path: &std::path::Path) -> Result<bool> {
    let (mut checks, t3_ok) = setup::check_host(cfg, path);
    if t3_ok {
        checks.extend(setup::check_t3(cfg, &t3::CliT3 { bin: cfg.t3_bin() }));
    }
    print!("{}", setup::render_checks(&checks));
    Ok(!checks.iter().any(|c| c.level == setup::Level::Fail))
}

const INSTALL_URL: &str = "https://raw.githubusercontent.com/cosmtrek/t3bot/main/scripts/install.sh";

/// Rerun the release installer into the directory this binary lives in.
fn upgrade(version: Option<String>, force: bool) -> Result<()> {
    let exe = std::env::current_exe().context("locating the t3bot binary")?;
    let dir = exe.parent().context("t3bot binary has no parent directory")?;
    let mut args = Vec::new();
    if let Some(v) = version {
        args.extend(["--version".to_string(), v]);
    }
    if force {
        args.push("--force".into());
    }
    let status = std::process::Command::new("sh")
        .arg("-c")
        .arg(r#"set -e; s=$(curl -fsSL "$0"); printf '%s\n' "$s" | sh -s -- "$@""#)
        .arg(INSTALL_URL)
        .args(&args)
        .env("T3BOT_BIN_DIR", dir)
        .status()
        .context("running sh (needs curl)")?;
    anyhow::ensure!(status.success(), "upgrade failed");
    Ok(())
}

fn warn_sync(rep: &sync::SyncReport) {
    for (p, e) in &rep.failed {
        eprintln!("t3bot: warning: listing {p} failed: {e}");
    }
}

pub fn run() -> Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Init { prompt: true, .. } => {
            print!("{}", cmd::tick::DEFAULT_PROMPT);
            Ok(())
        }
        Cmd::Init { print: true, .. } => {
            print!("{}", config::example());
            Ok(())
        }
        Cmd::Init { inbox, projects, provider, model, no_input, .. } => {
            let path = config::path();
            let existed = path.exists();
            let mut cfg = config::Config::load(&path)?;
            let before = cfg.clone();
            let mut args = InitArgs { inbox, provider, model, projects };
            if !no_input {
                setup::ask_missing(&cfg, &mut args)?;
            }
            setup::apply_init(&mut cfg, args)?;
            if let Some(bin) = setup::pin_t3_bin(&mut cfg) {
                println!("t3 is not on the timers' PATH; pinned t3_bin = {bin}");
            }
            if !existed || cfg != before {
                cfg.save(&path)?;
                println!("{} {}", if existed { "updated" } else { "wrote" }, path.display());
            } else {
                println!("{} unchanged", path.display());
            }
            doctor(&cfg, &path)?;
            Ok(())
        }
        Cmd::Config { cmd } => {
            let path = config::path();
            let mut cfg = config::Config::load(&path)?;
            match cmd.unwrap_or(ConfigCmd::Show) {
                ConfigCmd::Show => {
                    println!("# {}{}", path.display(), if path.exists() { "" } else { " (missing → t3bot init)" });
                    print!("{}", cfg.render());
                    return Ok(());
                }
                ConfigCmd::Path => {
                    println!("{}", path.display());
                    return Ok(());
                }
                ConfigCmd::Set { key, value } => {
                    let value = match key.as_str() {
                        "inbox_project" => config::expand_home(&config::normalize_project(&value)?),
                        "t3_bin" => config::expand_home(&value),
                        _ => value,
                    };
                    cfg.set(&key, Some(value))?
                }
                ConfigCmd::Unset { key } => cfg.set(&key, None)?,
                ConfigCmd::AddProject { paths } => {
                    setup::add_projects(&mut cfg, &paths)?;
                }
                ConfigCmd::RmProject { paths } => setup::rm_projects(&mut cfg, &paths)?,
            }
            cfg.save(&path)?;
            print!("{}", cfg.render());
            Ok(())
        }
        Cmd::Doctor => {
            let path = config::path();
            let cfg = config::Config::load(&path)?;
            if !doctor(&cfg, &path)? {
                std::process::exit(1);
            }
            Ok(())
        }
        Cmd::Upgrade { version, force } => upgrade(version, force),
        Cmd::Add { title, kind, topic, project, thread, next } => {
            let c = ctx()?;
            let id = cmd::add::add(&c, AddArgs { title, kind, topic, project, thread, next })?;
            println!("#{id} added");
            Ok(())
        }
        Cmd::Ls { topic, all, json } => {
            let c = ctx()?;
            warn_sync(&sync::sync(&c)?);
            print!("{}", cmd::ls::ls(&c, topic.as_deref(), all, json)?);
            Ok(())
        }
        Cmd::Edit { id, kind, topic, title, next, project } => {
            let c = ctx()?;
            let t = cmd::edit::edit(&c, id, EditArgs { kind, topic, title, next, project })?;
            println!("#{} {} [{}] {}", t.id, t.kind, t.topic.unwrap_or_default(), t.title);
            Ok(())
        }
        Cmd::Open { ids, say } => {
            let c = ctx()?;
            for o in cmd::open::open(&c, &parse_ids(&ids)?, say.as_deref())? {
                match o {
                    Opened::Sent { id, thread_id } => println!("#{id} → sent to {thread_id}"),
                    Opened::Started { id, thread_id } => println!("#{id} → started {thread_id}"),
                }
            }
            Ok(())
        }
        Cmd::Close { ids, drop, json } => {
            let c = ctx()?;
            let r = cmd::close::close(&c, &parse_ids(&ids)?, drop)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&r)?);
            } else {
                print!("{}", r.text());
            }
            cmd::close::ensure_found(&r)
        }
        Cmd::Report { json } => {
            let c = ctx()?;
            let rep = sync::sync(&c)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&cmd::report::items(&c)?)?);
            } else {
                print!("{}", cmd::report::render(&c, &rep)?);
            }
            Ok(())
        }
        Cmd::Pending { json: _, limit } => {
            let c = ctx()?;
            warn_sync(&sync::sync(&c)?);
            println!("{}", serde_json::to_string_pretty(&cmd::pending::pending(&c, limit)?)?);
            Ok(())
        }
        Cmd::Annotate { input } => {
            anyhow::ensure!(input == "-", "annotate reads JSON from stdin: use `t3bot annotate -`");
            let mut buf = String::new();
            std::io::stdin().read_to_string(&mut buf)?;
            let c = ctx()?;
            let r = cmd::annotate::annotate(&c, cmd::annotate::parse(&buf)?)?;
            println!("{}", serde_json::to_string(&r)?);
            Ok(())
        }
        Cmd::Sync => {
            let c = ctx()?;
            let r = sync::sync(&c)?;
            warn_sync(&r);
            println!(
                "seen {} · gone {} · dropped {} · auto-closed {} · pending {}",
                r.seen,
                r.gone,
                r.dropped,
                r.auto_closed,
                db::pending_threads(&c.db)?.len()
            );
            Ok(())
        }
        Cmd::Tick { always } => {
            let home = config::home();
            std::fs::create_dir_all(&home)?;
            let f = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(home.join("tick.lock"))?;
            let mut lock = fd_lock::RwLock::new(f);
            let Ok(_guard) = lock.try_write() else {
                println!("another tick is running; skipped");
                return Ok(());
            };
            let c = ctx()?;
            match cmd::tick::tick(&c, always, &cmd::tick::prompt_body())? {
                Tick::NoChange => println!("{} no change", c.now.strftime("%F %T")),
                Tick::Woke { inbox, created, pending } => println!(
                    "{} woke inbox {inbox}{} (pending {pending})",
                    c.now.strftime("%F %T"),
                    if created { " [new]" } else { "" }
                ),
            }
            Ok(())
        }
    }
}
